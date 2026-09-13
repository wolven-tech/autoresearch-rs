//! Local-only technical SEO evidence for frozen product-web routes.

use autoresearch_config::{ValidatedManifest, WebTargets};
use autoresearch_evaluator::EvaluationContext;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use thiserror::Error;
use url::Url;

const MAX_BODY_BYTES: u64 = 1024 * 1024;

/// SEO adapter failure. Failed technical rules remain in returned evidence.
#[derive(Debug, Error)]
pub enum SeoError {
    /// Manifest lacks frozen local web/SEO policy or route.
    #[error("SEO route or policy is not declared")]
    Undeclared,
    /// Redirect leaves local origin or lands on undeclared path.
    #[error("redirect target is not a declared local route")]
    UnsafeRedirect,
    /// Redirect chain exceeded frozen bound.
    #[error("redirect chain exceeded frozen bound")]
    RedirectLimit,
    /// Local HTTP probe failed.
    #[error("local SEO request failed")]
    Network,
    /// Static selector could not be parsed.
    #[error("SEO selector unavailable")]
    Parse,
    /// Run-owned artifact could not be written safely.
    #[error("SEO artifact unavailable")]
    Artifact,
}

/// One explicitly followed document redirect or final response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeoHop {
    /// Exact checked URL.
    pub url: String,
    /// HTTP response status at this URL.
    pub status: u16,
}

/// One failing technical rule, not an indexation or ranking assertion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeoIssue {
    /// Stable rule identifier.
    pub rule_id: String,
    /// Manifest route name.
    pub route: String,
    /// Exact source URL checked.
    pub source_url: String,
    /// Run-owned JSON artifact holding bounded evidence.
    pub source_artifact: String,
    /// Short machine-derived explanation.
    pub detail: String,
}

/// Bounded route-level technical SEO evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeoEvidence {
    /// Frozen route name.
    pub route: String,
    /// Initial declared URL.
    pub source_url: String,
    /// Final declared URL after bounded redirects.
    pub final_url: String,
    /// Observed HTTP chain.
    pub hops: Vec<SeoHop>,
    /// Parsed canonical, when present.
    pub canonical: Option<String>,
    /// Parsed robots meta, when present.
    pub robots_meta: Option<String>,
    /// Number of parsed JSON-LD blocks.
    pub json_ld_blocks: u32,
    /// Run-owned source artifact.
    pub artifact_relative_path: String,
    /// Failing rules only; absence does not prove ranking or indexation.
    pub issues: Vec<SeoIssue>,
}

pub(crate) struct Fetched {
    pub(crate) hops: Vec<SeoHop>,
    pub(crate) final_url: Url,
    pub(crate) body: String,
}

/// Checks one declared local route and writes a bounded evidence artifact.
///
/// No remote origin is supported. A future remote adapter needs an explicit
/// read-only allowlist and per-run network authority. This function never
/// claims Google indexation, rankings, or commercial validation.
///
/// # Errors
///
/// Rejects undeclared paths, off-origin redirects, transport failures, and
/// unsafe artifact writes. Technical rule failures return `SeoIssue`s.
pub fn inspect_local_route(
    context: &EvaluationContext,
    manifest: &ValidatedManifest,
    route_name: &str,
) -> Result<SeoEvidence, SeoError> {
    let web = manifest.web().ok_or(SeoError::Undeclared)?;
    let seo = web.seo().ok_or(SeoError::Undeclared)?;
    let (route_index, route) = web
        .routes()
        .iter()
        .enumerate()
        .find(|(_, route)| route.name() == route_name)
        .ok_or(SeoError::Undeclared)?;
    let origin = Url::parse(web.origin()).map_err(|_| SeoError::Undeclared)?;
    let source_url = origin
        .join(route.path())
        .map_err(|_| SeoError::Undeclared)?;
    let agent = local_agent();
    let fetched = fetch_declared_chain(&agent, web, &source_url, seo.max_redirects())?;
    let artifact = format!("seo/route-{route_index:03}.json");
    let document = Html::parse_document(&fetched.body);
    let canonical = first_attr(&document, "link[rel~=canonical]", "href")?;
    let robots_meta = first_attr(&document, "meta[name=robots]", "content")?;
    let json_ld_blocks = count_json_ld(&document)?;
    let mut evidence = SeoEvidence {
        route: route_name.into(),
        source_url: source_url.to_string(),
        final_url: fetched.final_url.to_string(),
        hops: fetched.hops,
        canonical,
        robots_meta,
        json_ld_blocks,
        artifact_relative_path: artifact.clone(),
        issues: Vec::new(),
    };
    let first_status = evidence.hops.first().map_or(0, |hop| hop.status);
    if first_status != route.expected_status() {
        issue(
            &mut evidence,
            "http_status",
            format!(
                "expected {}, observed {first_status}",
                route.expected_status()
            ),
        );
    }
    if route.indexable() {
        if evidence.canonical.as_deref() != Some(evidence.final_url.as_str()) {
            issue(
                &mut evidence,
                "canonical_mismatch",
                "canonical differs from final URL".into(),
            );
        }
        if evidence.robots_meta.as_deref().is_some_and(has_noindex) {
            issue(
                &mut evidence,
                "unexpected_noindex",
                "indexable route declares noindex".into(),
            );
        }
    }
    check_json_ld(&document, &mut evidence)?;
    check_robots_and_sitemap(&agent, web, &origin, &mut evidence)?;
    write_artifact(context.artifact_directory(), &evidence)?;
    Ok(evidence)
}

pub(crate) fn local_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .into()
}

pub(crate) fn fetch_declared_chain(
    agent: &ureq::Agent,
    web: &WebTargets,
    source: &Url,
    max_redirects: u8,
) -> Result<Fetched, SeoError> {
    let mut current = source.clone();
    let mut hops = Vec::new();
    loop {
        let mut response = agent
            .get(current.as_str())
            .call()
            .map_err(|_| SeoError::Network)?;
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        hops.push(SeoHop {
            url: current.to_string(),
            status,
        });
        if (300..400).contains(&status) && location.is_some() {
            if hops.len() > usize::from(max_redirects) {
                return Err(SeoError::RedirectLimit);
            }
            let target = current
                .join(location.as_deref().unwrap_or_default())
                .map_err(|_| SeoError::UnsafeRedirect)?;
            if target.origin() != current.origin()
                || target.query().is_some()
                || target.fragment().is_some()
                || !web
                    .routes()
                    .iter()
                    .any(|route| route.path() == target.path())
            {
                return Err(SeoError::UnsafeRedirect);
            }
            current = target;
            continue;
        }
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY_BYTES)
            .read_to_string()
            .map_err(|_| SeoError::Network)?;
        return Ok(Fetched {
            hops,
            final_url: current,
            body,
        });
    }
}

fn first_attr(
    document: &Html,
    selector: &str,
    attribute: &str,
) -> Result<Option<String>, SeoError> {
    let selector = Selector::parse(selector).map_err(|_| SeoError::Parse)?;
    Ok(document
        .select(&selector)
        .next()
        .and_then(|element| element.value().attr(attribute))
        .map(str::to_owned))
}

fn count_json_ld(document: &Html) -> Result<u32, SeoError> {
    let selector =
        Selector::parse("script[type='application/ld+json']").map_err(|_| SeoError::Parse)?;
    Ok(u32::try_from(document.select(&selector).count()).unwrap_or(u32::MAX))
}

fn check_json_ld(document: &Html, evidence: &mut SeoEvidence) -> Result<(), SeoError> {
    let selector =
        Selector::parse("script[type='application/ld+json']").map_err(|_| SeoError::Parse)?;
    for (index, block) in document.select(&selector).enumerate() {
        if serde_json::from_str::<serde_json::Value>(&block.text().collect::<String>()).is_err() {
            issue(
                evidence,
                "malformed_json_ld",
                format!("block {index} is not valid JSON"),
            );
        }
    }
    Ok(())
}

fn check_robots_and_sitemap(
    agent: &ureq::Agent,
    web: &WebTargets,
    origin: &Url,
    evidence: &mut SeoEvidence,
) -> Result<(), SeoError> {
    let seo = web.seo().ok_or(SeoError::Undeclared)?;
    let robots_url = origin
        .join(seo.robots_path())
        .map_err(|_| SeoError::Undeclared)?;
    let sitemap_url = origin
        .join(seo.sitemap_path())
        .map_err(|_| SeoError::Undeclared)?;
    let mut robots = agent
        .get(robots_url.as_str())
        .call()
        .map_err(|_| SeoError::Network)?;
    if robots.status().as_u16() == 200 {
        let body = robots
            .body_mut()
            .with_config()
            .limit(MAX_BODY_BYTES)
            .read_to_string()
            .map_err(|_| SeoError::Network)?;
        if !body
            .lines()
            .any(|line| line.trim() == format!("Sitemap: {sitemap_url}"))
        {
            issue(
                evidence,
                "robots_sitemap_missing",
                "robots file omits declared sitemap URL".into(),
            );
        }
    } else {
        issue(
            evidence,
            "robots_unavailable",
            "declared robots path did not return 200".into(),
        );
    }
    let mut sitemap = agent
        .get(sitemap_url.as_str())
        .call()
        .map_err(|_| SeoError::Network)?;
    if sitemap.status().as_u16() != 200 {
        issue(
            evidence,
            "sitemap_unavailable",
            "declared sitemap path did not return 200".into(),
        );
    } else if web
        .routes()
        .iter()
        .any(|route| route.indexable() && evidence.final_url.ends_with(route.path()))
    {
        let body = sitemap
            .body_mut()
            .with_config()
            .limit(MAX_BODY_BYTES)
            .read_to_string()
            .map_err(|_| SeoError::Network)?;
        if !body.contains(&format!("<loc>{}</loc>", evidence.final_url)) {
            issue(
                evidence,
                "sitemap_url_missing",
                "indexable URL missing from declared sitemap".into(),
            );
        }
    }
    Ok(())
}

fn has_noindex(value: &str) -> bool {
    value
        .split(',')
        .any(|token| token.trim().eq_ignore_ascii_case("noindex"))
}

fn issue(evidence: &mut SeoEvidence, rule_id: &str, detail: String) {
    evidence.issues.push(SeoIssue {
        rule_id: rule_id.into(),
        route: evidence.route.clone(),
        source_url: evidence.source_url.clone(),
        source_artifact: evidence.artifact_relative_path.clone(),
        detail,
    });
}

fn write_artifact(root: &Path, evidence: &SeoEvidence) -> Result<(), SeoError> {
    let directory = root.join("seo");
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&directory).map_err(|_| SeoError::Artifact)?;
        }
        Ok(_) | Err(_) => return Err(SeoError::Artifact),
    }
    let serialized = serde_json::to_vec_pretty(evidence).map_err(|_| SeoError::Artifact)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(root.join(&evidence.artifact_relative_path))
        .map_err(|_| SeoError::Artifact)?;
    file.write_all(&serialized).map_err(|_| SeoError::Artifact)
}
