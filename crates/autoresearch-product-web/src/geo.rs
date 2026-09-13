//! Bounded, source-grounded GEO diagnostics for declared local pages.

use crate::seo::{SeoError, fetch_declared_chain, local_agent};
use autoresearch_config::{GeoSettings, ValidatedManifest, WebTargets};
use autoresearch_evaluator::EvaluationContext;
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use thiserror::Error;
use url::Url;

const MAX_ENTITY_NAMES: usize = 16;
const MAX_LINKS_PER_PASSAGE: usize = 32;
const MAX_CLAIMS_PER_PASSAGE: usize = 32;
const MAX_FACT_OBSERVATIONS: usize = 64;

/// Local GEO adapter error; rule failures return diagnostics instead.
#[derive(Debug, Error)]
pub enum GeoError {
    /// Route or frozen policy is missing.
    #[error("GEO route or policy is not declared")]
    Undeclared,
    /// Local route cannot be safely fetched.
    #[error(transparent)]
    Route(#[from] SeoError),
    /// Run-owned evidence artifact cannot be written.
    #[error("GEO artifact unavailable")]
    Artifact,
    /// Static selector cannot be parsed.
    #[error("GEO selector unavailable")]
    Selector,
    /// Input exceeds bounded diagnostic policy; no partial pass is returned.
    #[error("GEO source exceeds bounded diagnostic policy")]
    EvidenceLimit,
}

/// One bounded source-HTML passage from the actual local page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeoPassage {
    /// Stable author-supplied passage identifier.
    pub id: String,
    /// Source-HTML text excerpt, capped at 500 Unicode scalar values.
    pub excerpt: String,
    /// Observed citation links, never automatically fetched or verified.
    pub source_links: Vec<String>,
}

/// Diagnostic anchored to real source URL, passage, rule, and artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeoIssue {
    /// Stable diagnostic rule.
    pub rule_id: String,
    /// Frozen manifest route.
    pub route: String,
    /// Exact local URL inspected.
    pub source_url: String,
    /// Passage identifier or `document` for document-wide metadata.
    pub passage_id: String,
    /// Actual source text excerpt, never generated prose.
    pub source_passage: String,
    /// Run-owned source artifact.
    pub source_artifact: String,
}

/// Local technical GEO evidence. No live search visibility is inferred.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeoEvidence {
    /// Frozen route name.
    pub route: String,
    /// Final declared URL inspected.
    pub source_url: String,
    /// Observed entity names from page metadata or JSON-LD.
    pub entity_names: Vec<String>,
    /// Author-annotated source-HTML passages, bounded by manifest.
    pub passages: Vec<GeoPassage>,
    /// Diagnostic failures.
    pub issues: Vec<GeoIssue>,
    /// Run-owned JSON artifact path.
    pub artifact_relative_path: String,
}

/// Inspects declared local HTML for citation-ready passages and fact consistency.
///
/// Annotation checks establish only source coverage and internal consistency.
/// They are not AI-generated answers, live citations, impressions, receipts,
/// or evidence that a product bet gate passed.
///
/// # Errors
///
/// Rejects undeclared routes and unsafe local fetches or artifact writes.
pub fn inspect_local_geo_route(
    context: &EvaluationContext,
    manifest: &ValidatedManifest,
    route_name: &str,
) -> Result<GeoEvidence, GeoError> {
    let web = manifest.web().ok_or(GeoError::Undeclared)?;
    let policy = web.geo().ok_or(GeoError::Undeclared)?;
    let seo = web.seo().ok_or(GeoError::Undeclared)?;
    let (route_index, route) = web
        .routes()
        .iter()
        .enumerate()
        .find(|(_, route)| {
            route.name() == route_name && route.indexable() && route.expected_status() == 200
        })
        .ok_or(GeoError::Undeclared)?;
    let source = Url::parse(web.origin())
        .map_err(|_| GeoError::Undeclared)?
        .join(route.path())
        .map_err(|_| GeoError::Undeclared)?;
    let fetched = fetch_declared_chain(&local_agent(), web, &source, seo.max_redirects())?;
    let document = Html::parse_document(&fetched.body);
    let artifact = format!("geo/route-{route_index:03}.json");
    let mut evidence = GeoEvidence {
        route: route_name.into(),
        source_url: fetched.final_url.to_string(),
        entity_names: Vec::new(),
        passages: Vec::new(),
        issues: Vec::new(),
        artifact_relative_path: artifact,
    };
    if fetched.hops.last().is_none_or(|hop| hop.status != 200) {
        add_issue(&mut evidence, "http_status", "document", "non-200 response");
    }
    inspect_entity(&document, policy, &mut evidence)?;
    inspect_passages(&document, web, policy, &mut evidence)?;
    inspect_facts(&document, policy, &mut evidence)?;
    write_artifact(context.artifact_directory(), &evidence)?;
    Ok(evidence)
}

fn inspect_entity(
    document: &Html,
    policy: &GeoSettings,
    evidence: &mut GeoEvidence,
) -> Result<(), GeoError> {
    let meta = Selector::parse("meta[property='og:site_name']").map_err(|_| GeoError::Selector)?;
    for element in document.select(&meta) {
        if let Some(name) = element.value().attr("content") {
            if name.len() > 120 {
                return Err(GeoError::EvidenceLimit);
            }
            evidence.entity_names.push(name.trim().to_owned());
        }
    }
    if evidence.entity_names.len() > MAX_ENTITY_NAMES {
        return Err(GeoError::EvidenceLimit);
    }
    let scripts =
        Selector::parse("script[type='application/ld+json']").map_err(|_| GeoError::Selector)?;
    for script in document.select(&scripts) {
        if let Ok(value) =
            serde_json::from_str::<serde_json::Value>(&script.text().collect::<String>())
        {
            collect_entity_names(&value, &mut evidence.entity_names);
        }
        if evidence.entity_names.len() > MAX_ENTITY_NAMES
            || evidence.entity_names.iter().any(|name| name.len() > 120)
        {
            return Err(GeoError::EvidenceLimit);
        }
    }
    if evidence.entity_names.is_empty() {
        add_issue(
            evidence,
            "entity_name_missing",
            "document",
            "no entity name in metadata",
        );
    }
    let conflicts: Vec<String> = evidence
        .entity_names
        .iter()
        .filter(|name| name.as_str() != policy.entity_name())
        .cloned()
        .collect();
    for name in conflicts {
        add_issue(evidence, "entity_name_conflict", "document", &name);
    }
    Ok(())
}

fn collect_entity_names(value: &serde_json::Value, names: &mut Vec<String>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_entity_names(item, names);
            }
        }
        serde_json::Value::Object(map) => {
            let entity_type = map.get("@type").and_then(serde_json::Value::as_str);
            if matches!(
                entity_type,
                Some("Organization" | "WebSite" | "SoftwareApplication")
            ) && let Some(name) = map.get("name").and_then(serde_json::Value::as_str)
            {
                names.push(name.trim().to_owned());
            }
            if let Some(graph) = map.get("@graph") {
                collect_entity_names(graph, names);
            }
        }
        _ => {}
    }
}

fn inspect_passages(
    document: &Html,
    web: &WebTargets,
    policy: &GeoSettings,
    evidence: &mut GeoEvidence,
) -> Result<(), GeoError> {
    let selector = Selector::parse("[data-geo-passage]").map_err(|_| GeoError::Selector)?;
    let claim_selector = Selector::parse("[data-geo-claim]").map_err(|_| GeoError::Selector)?;
    let link_selector = Selector::parse("a[id][href]").map_err(|_| GeoError::Selector)?;
    let all_claims = document.select(&claim_selector).collect::<Vec<_>>();
    if all_claims.len() > usize::from(policy.max_passages()) * MAX_CLAIMS_PER_PASSAGE {
        return Err(GeoError::EvidenceLimit);
    }
    for claim in all_claims {
        if !claim
            .ancestors()
            .filter_map(ElementRef::wrap)
            .any(|ancestor| ancestor.value().attr("data-geo-passage").is_some())
        {
            add_issue(
                evidence,
                "claim_outside_passage",
                "document",
                &text_excerpt(claim),
            );
        }
    }
    let passages: Vec<_> = document.select(&selector).collect();
    if passages.len() > usize::from(policy.max_passages()) {
        return Err(GeoError::EvidenceLimit);
    }
    for (index, passage) in passages
        .into_iter()
        .take(usize::from(policy.max_passages()))
        .enumerate()
    {
        let id = passage
            .value()
            .attr("data-geo-passage")
            .unwrap_or("")
            .trim();
        let id = if id.is_empty() {
            format!("passage-{index}")
        } else {
            id.to_owned()
        };
        if id.len() > 80 {
            return Err(GeoError::EvidenceLimit);
        }
        let excerpt = text_excerpt(passage);
        let links = passage
            .select(&link_selector)
            .filter_map(|link| link.value().attr("href"))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if links.len() > MAX_LINKS_PER_PASSAGE || links.iter().any(|link| link.len() > 500) {
            return Err(GeoError::EvidenceLimit);
        }
        evidence.passages.push(GeoPassage {
            id: id.clone(),
            excerpt,
            source_links: links,
        });
        let claims = passage.select(&claim_selector).collect::<Vec<_>>();
        if claims.len() > MAX_CLAIMS_PER_PASSAGE {
            return Err(GeoError::EvidenceLimit);
        }
        for claim in claims {
            let claim_excerpt = text_excerpt(claim);
            match claim
                .value()
                .attr("data-geo-source-ref")
                .filter(|value| !value.trim().is_empty())
            {
                None => add_issue(evidence, "uncited_claim", &id, &claim_excerpt),
                Some(source_ref) if !valid_source_ref(passage, &link_selector, web, source_ref) => {
                    add_issue(evidence, "missing_source_reference", &id, &claim_excerpt);
                }
                Some(_) => {}
            }
        }
    }
    Ok(())
}

fn valid_source_ref(
    passage: ElementRef<'_>,
    selector: &Selector,
    web: &WebTargets,
    source_ref: &str,
) -> bool {
    passage.select(selector).any(|link| {
        if link.value().attr("id") != Some(source_ref) {
            return false;
        }
        let Some(href) = link.value().attr("href") else {
            return false;
        };
        if href.len() > 500 {
            return false;
        }
        if href.starts_with('/') {
            return web.routes().iter().any(|route| route.path() == href);
        }
        Url::parse(href).is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some())
    })
}

fn inspect_facts(
    document: &Html,
    policy: &GeoSettings,
    evidence: &mut GeoEvidence,
) -> Result<(), GeoError> {
    let selector = Selector::parse("[data-geo-fact]").map_err(|_| GeoError::Selector)?;
    let mut observed: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let facts = document.select(&selector).collect::<Vec<_>>();
    if facts.len() > MAX_FACT_OBSERVATIONS {
        return Err(GeoError::EvidenceLimit);
    }
    for element in facts {
        if let Some(key) = element.value().attr("data-geo-fact") {
            observed
                .entry(key.to_owned())
                .or_default()
                .insert(text_excerpt(element));
        }
    }
    for (key, expected) in policy.facts() {
        match observed.get(key) {
            None => add_issue(evidence, "product_fact_missing", "document", key),
            Some(values) => {
                for value in values {
                    if value != expected {
                        add_issue(evidence, "product_fact_conflict", "document", value);
                    }
                }
            }
        }
    }
    Ok(())
}

fn text_excerpt(element: ElementRef<'_>) -> String {
    let normalized = element
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    normalized.chars().take(500).collect()
}

fn add_issue(evidence: &mut GeoEvidence, rule_id: &str, passage_id: &str, excerpt: &str) {
    evidence.issues.push(GeoIssue {
        rule_id: rule_id.into(),
        route: evidence.route.clone(),
        source_url: evidence.source_url.clone(),
        passage_id: passage_id.into(),
        source_passage: excerpt.into(),
        source_artifact: evidence.artifact_relative_path.clone(),
    });
}

fn write_artifact(root: &Path, evidence: &GeoEvidence) -> Result<(), GeoError> {
    let directory = root.join("geo");
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&directory).map_err(|_| GeoError::Artifact)?;
        }
        Ok(_) | Err(_) => return Err(GeoError::Artifact),
    }
    let serialized = serde_json::to_vec_pretty(evidence).map_err(|_| GeoError::Artifact)?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(root.join(&evidence.artifact_relative_path))
        .map_err(|_| GeoError::Artifact)?;
    file.write_all(&serialized).map_err(|_| GeoError::Artifact)
}
