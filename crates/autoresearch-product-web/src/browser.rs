//! Headless Chromium inspection of rendered local product pages.

use autoresearch_config::ValidatedManifest;
use autoresearch_core::{GateOutcome, Measurement};
use autoresearch_evaluator::{
    Artifact, EvaluationContext, EvaluatorOutput, OutputError, ValidatedOutput, validate_output,
};
use headless_chrome::{
    Browser, LaunchOptions,
    browser::tab::RequestPausedDecision,
    protocol::cdp::{Emulation, Fetch, Log, Network, Page, Runtime, types::Event},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use thiserror::Error;
use url::Url;

const MAX_SCREENSHOT_BYTES: usize = 12 * 1024 * 1024;
const VIEWPORT_HEIGHT: u32 = 800;

/// Invalid target, browser failure, or unsafe screenshot output.
#[derive(Debug, Error)]
pub enum BrowserError {
    /// Only loopback HTTP or candidate-contained files are accepted without run authority.
    #[error("browser target must be loopback HTTP or candidate-contained file URL")]
    NonLocalTarget,
    /// Browser executable does not resolve to a regular file.
    #[error("Chromium executable unavailable")]
    ChromiumUnavailable,
    /// Manifest does not define product-web targets.
    #[error("manifest has no product-web targets")]
    MissingWebTargets,
    /// Requested route is not frozen in manifest.
    #[error("product-web route is not declared")]
    UnknownRoute,
    /// Viewport configuration is invalid or duplicated.
    #[error("viewport widths must be unique and within 240..=4096")]
    InvalidViewport,
    /// Chromium could not launch.
    #[error("Chromium launch failed")]
    Launch,
    /// Browser navigation failed.
    #[error("browser navigation failed")]
    Navigation,
    /// Rendered DOM could not be inspected.
    #[error("rendered browser evidence unavailable")]
    Inspection,
    /// Screenshot escaped or exceeded run-owned artifact policy.
    #[error("browser screenshot artifact unsafe or unavailable")]
    Artifact,
}

/// One rendered viewport, not a claim of full WCAG conformance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewportEvidence {
    /// Requested CSS viewport width.
    pub requested_width: u32,
    /// Browser-observed inner width.
    pub observed_width: u32,
    /// Document width including horizontal overflow.
    pub document_width: u32,
    /// Main document HTTP status, absent for `file://` fixtures.
    pub http_status: Option<u16>,
    /// Runtime console error calls observed before capture.
    pub console_errors: u32,
    /// Uncaught JavaScript exceptions observed before capture.
    pub runtime_exceptions: u32,
    /// Browser log error entries observed before capture.
    pub log_errors: u32,
    /// Visible elements extending beyond right viewport edge (bounded sample).
    pub overflow_elements: Vec<String>,
    /// Visible interactive controls without an accessible-name approximation.
    pub unnamed_controls: Vec<String>,
    /// Requests outside loopback or candidate files blocked before dispatch.
    pub blocked_nonlocal_requests: u32,
    /// Blocked HTTP(S) requests to non-loopback hosts.
    pub blocked_external_network_requests: u32,
    /// Screenshot path relative to run-owned artifact directory.
    pub screenshot_relative_path: String,
}

/// Browser-observed page identity and viewport diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserEvidence {
    /// Frozen route name when invoked through manifest; absent for direct file probe.
    pub route_name: Option<String>,
    /// Frozen expected HTTP status when invoked through manifest.
    pub expected_http_status: Option<u16>,
    /// Browser-observed final URL after navigation.
    pub final_url: String,
    /// Rendered page title.
    pub title: String,
    /// Document language declaration.
    pub language: String,
    /// Rendered H1 count.
    pub h1_count: u32,
    /// Rendered meta description, if any.
    pub description: Option<String>,
    /// Rendered canonical URL, if any.
    pub canonical: Option<String>,
    /// Rendered robots directives, if any.
    pub robots: Option<String>,
    /// One observation per requested viewport.
    pub viewports: Vec<ViewportEvidence>,
}

/// Converts rendered evidence into four declared hard gates and screenshots.
///
/// Gate names are `browser_viewport_exact`, `browser_no_overflow`,
/// `browser_controls_named`, and `browser_local_only`. These checks are
/// deliberately narrower than WCAG AA, Lighthouse, SEO, or market evidence.
/// Frozen manifest must declare these exact names before a snapshot can be
/// built. Blocked external requests fail local-only gate even though fetch
/// interception prevented dispatch.
///
/// # Errors
///
/// Returns shared structural validation failure for mismatched context or
/// unsafe artifact paths.
pub fn browser_output(
    context: &EvaluationContext,
    evaluator_id: &str,
    evidence: &BrowserEvidence,
) -> Result<ValidatedOutput, OutputError> {
    let mut gates = vec![
        (
            "browser_viewport_exact",
            evidence
                .viewports
                .iter()
                .all(|viewport| viewport.observed_width == viewport.requested_width),
        ),
        (
            "browser_no_overflow",
            evidence.viewports.iter().all(|viewport| {
                viewport.document_width <= viewport.requested_width
                    && viewport.overflow_elements.is_empty()
            }),
        ),
        (
            "browser_controls_named",
            evidence
                .viewports
                .iter()
                .all(|viewport| viewport.unnamed_controls.is_empty()),
        ),
        (
            "browser_local_only",
            evidence
                .viewports
                .iter()
                .all(|viewport| viewport.blocked_nonlocal_requests == 0),
        ),
    ];
    if let Some(expected) = evidence.expected_http_status {
        gates.push((
            "browser_route_status",
            evidence
                .viewports
                .iter()
                .all(|viewport| viewport.http_status == Some(expected)),
        ));
    }
    let measurements = gates
        .into_iter()
        .map(|(name, passed)| Measurement::HardGate {
            name: name.into(),
            outcome: GateOutcome::new(passed, None),
        })
        .collect();
    let artifacts = evidence
        .viewports
        .iter()
        .map(|viewport| Artifact {
            name: format!("viewport_{}", viewport.requested_width),
            relative_path: viewport.screenshot_relative_path.clone(),
            media_type: "image/png".into(),
        })
        .collect();
    validate_output(
        context,
        evaluator_id,
        EvaluatorOutput {
            evaluator_id: evaluator_id.into(),
            run_id: context.run_id().to_string(),
            baseline_commit: context.baseline_commit().to_string(),
            evaluated_commit: context.evaluated_commit().to_string(),
            measurements,
            observations: vec![],
            artifacts,
            warnings: vec![],
        },
    )
}

#[derive(Debug, Deserialize)]
struct DomEvidence {
    title: String,
    language: String,
    h1_count: u32,
    description: Option<String>,
    canonical: Option<String>,
    robots: Option<String>,
    observed_width: u32,
    document_width: u32,
    overflow_elements: Vec<String>,
    unnamed_controls: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct PageIdentity {
    final_url: String,
    title: String,
    language: String,
    h1_count: u32,
    description: Option<String>,
    canonical: Option<String>,
    robots: Option<String>,
}

/// Inspects one local route at declared widths and writes bounded screenshots.
///
/// Browser result is diagnostic evidence only. It does not certify WCAG AA,
/// SEO ranking, conversion, or market demand. Caller must run Chromium under
/// an OS-level network/filesystem sandbox for hostile pages; SDK restricts
/// local targets, intercepts nonlocal page requests, and owns artifact paths.
///
/// # Errors
///
/// Rejects non-loopback targets, invalid Chromium/viewport config, browser
/// failures, and unsafe screenshot writes.
pub fn inspect_local_page(
    context: &EvaluationContext,
    target: &str,
    chromium_path: &Path,
    widths: &[u32],
) -> Result<BrowserEvidence, BrowserError> {
    inspect_local_page_with_settings(context, target, chromium_path, widths, true)
}

/// Inspects one manifest-declared loopback route with frozen capture settings.
///
/// # Errors
///
/// Rejects missing route settings, unsafe targets, unavailable Chromium, and
/// browser/artifact failures. A returned unexpected HTTP status is recorded as
/// a failed `browser_route_status` hard gate rather than a successful result.
pub fn inspect_declared_route(
    context: &EvaluationContext,
    manifest: &ValidatedManifest,
    route_name: &str,
    chromium_path: &Path,
) -> Result<BrowserEvidence, BrowserError> {
    let web = manifest.web().ok_or(BrowserError::MissingWebTargets)?;
    let route = web
        .routes()
        .iter()
        .find(|route| route.name() == route_name)
        .ok_or(BrowserError::UnknownRoute)?;
    let url = Url::parse(web.origin())
        .and_then(|origin| origin.join(route.path()))
        .map_err(|_| BrowserError::NonLocalTarget)?;
    let mut evidence = inspect_local_page_with_settings(
        context,
        url.as_str(),
        chromium_path,
        web.viewports(),
        web.reduced_motion(),
    )?;
    evidence.route_name = Some(route.name().to_owned());
    evidence.expected_http_status = Some(route.expected_status());
    Ok(evidence)
}

fn inspect_local_page_with_settings(
    context: &EvaluationContext,
    target: &str,
    chromium_path: &Path,
    widths: &[u32],
    reduced_motion: bool,
) -> Result<BrowserEvidence, BrowserError> {
    validate_local_target(target, context.candidate_worktree())?;
    let chromium =
        fs::canonicalize(chromium_path).map_err(|_| BrowserError::ChromiumUnavailable)?;
    if !chromium.is_file() {
        return Err(BrowserError::ChromiumUnavailable);
    }
    let unique = widths.iter().copied().collect::<BTreeSet<_>>();
    if widths.is_empty()
        || unique.len() != widths.len()
        || widths.iter().any(|width| !(240..=4096).contains(width))
    {
        return Err(BrowserError::InvalidViewport);
    }
    let screenshot_root = create_artifact_directory(context.artifact_directory())?;
    let mut viewports = Vec::with_capacity(widths.len());
    let mut identity: Option<PageIdentity> = None;
    for &width in widths {
        let (viewport, page) = inspect_viewport(
            context,
            target,
            &chromium,
            width,
            reduced_motion,
            &screenshot_root,
        )?;
        if identity.as_ref().is_some_and(|first| first != &page) {
            return Err(BrowserError::Inspection);
        }
        identity = Some(page);
        viewports.push(viewport);
    }
    let page = identity.ok_or(BrowserError::Inspection)?;
    Ok(BrowserEvidence {
        route_name: None,
        expected_http_status: None,
        final_url: page.final_url,
        title: page.title,
        language: page.language,
        h1_count: page.h1_count,
        description: page.description,
        canonical: page.canonical,
        robots: page.robots,
        viewports,
    })
}

fn inspect_viewport(
    context: &EvaluationContext,
    target: &str,
    chromium: &Path,
    width: u32,
    reduced_motion: bool,
    screenshot_root: &Path,
) -> Result<(ViewportEvidence, PageIdentity), BrowserError> {
    let browser = Browser::new(
        LaunchOptions::default_builder()
            .path(Some(chromium.to_path_buf()))
            .window_size(Some((width, VIEWPORT_HEIGHT)))
            .ignore_certificate_errors(false)
            .build()
            .map_err(|_| BrowserError::Launch)?,
    )
    .map_err(|_| BrowserError::Launch)?;
    let tab = browser.new_tab().map_err(|_| BrowserError::Launch)?;
    tab.set_default_timeout(Duration::from_secs(15));
    configure_viewport(&tab, width, reduced_motion)?;
    let observers = attach_page_observers(&tab)?;
    let guard = attach_local_guard(&tab, context.candidate_worktree())?;
    tab.navigate_to(target)
        .and_then(headless_chrome::browser::tab::Tab::wait_until_navigated)
        .map_err(|_| BrowserError::Navigation)?;
    let final_url = tab.get_url();
    validate_local_target(&final_url, context.candidate_worktree())?;
    let dom = read_dom(&tab, width)?;
    let screenshot_relative_path = save_screenshot(&tab, screenshot_root, width)?;
    let page = PageIdentity {
        final_url,
        title: dom.title,
        language: dom.language,
        h1_count: dom.h1_count,
        description: dom.description,
        canonical: dom.canonical,
        robots: dom.robots,
    };
    let viewport = ViewportEvidence {
        requested_width: width,
        observed_width: dom.observed_width,
        document_width: dom.document_width,
        http_status: u16::try_from(observers.http_status.load(Ordering::Relaxed))
            .ok()
            .filter(|value| *value != 0),
        console_errors: observers.console_errors.load(Ordering::Relaxed),
        runtime_exceptions: observers.runtime_exceptions.load(Ordering::Relaxed),
        log_errors: observers.log_errors.load(Ordering::Relaxed),
        overflow_elements: dom.overflow_elements,
        unnamed_controls: dom.unnamed_controls,
        blocked_nonlocal_requests: guard.blocked.load(Ordering::Relaxed),
        blocked_external_network_requests: guard.external.load(Ordering::Relaxed),
        screenshot_relative_path,
    };
    Ok((viewport, page))
}

fn configure_viewport(
    tab: &headless_chrome::Tab,
    width: u32,
    reduced_motion: bool,
) -> Result<(), BrowserError> {
    tab.call_method(Emulation::SetDeviceMetricsOverride {
        width,
        height: VIEWPORT_HEIGHT,
        device_scale_factor: 1.0,
        mobile: width <= 390,
        scale: None,
        screen_width: Some(width),
        screen_height: Some(VIEWPORT_HEIGHT),
        position_x: None,
        position_y: None,
        dont_set_visible_size: None,
        screen_orientation: None,
        viewport: None,
        display_feature: None,
        device_posture: None,
    })
    .map_err(|_| BrowserError::Launch)?;
    tab.call_method(Emulation::SetEmulatedMedia {
        media: None,
        features: Some(vec![Emulation::MediaFeature {
            name: "prefers-reduced-motion".into(),
            value: if reduced_motion {
                "reduce"
            } else {
                "no-preference"
            }
            .into(),
        }]),
    })
    .map_err(|_| BrowserError::Launch)?;
    Ok(())
}

struct PageObservers {
    http_status: Arc<AtomicU32>,
    console_errors: Arc<AtomicU32>,
    runtime_exceptions: Arc<AtomicU32>,
    log_errors: Arc<AtomicU32>,
}

fn attach_page_observers(tab: &headless_chrome::Tab) -> Result<PageObservers, BrowserError> {
    let http_status = Arc::new(AtomicU32::new(0));
    let status_for_handler = Arc::clone(&http_status);
    tab.register_response_handling(
        "main_document_status",
        Box::new(move |params, _| {
            if matches!(params.Type, Network::ResourceType::Document) {
                status_for_handler.store(params.response.status, Ordering::Relaxed);
            }
        }),
    )
    .map_err(|_| BrowserError::Launch)?;
    let console_errors = Arc::new(AtomicU32::new(0));
    let runtime_exceptions = Arc::new(AtomicU32::new(0));
    let log_errors = Arc::new(AtomicU32::new(0));
    let console_for_listener = Arc::clone(&console_errors);
    let exceptions_for_listener = Arc::clone(&runtime_exceptions);
    let log_for_listener = Arc::clone(&log_errors);
    tab.enable_runtime().map_err(|_| BrowserError::Launch)?;
    tab.enable_log().map_err(|_| BrowserError::Launch)?;
    tab.add_event_listener(Arc::new(move |event: &Event| match event {
        Event::RuntimeConsoleAPICalled(entry)
            if matches!(
                entry.params.Type,
                Runtime::ConsoleAPICalledEventTypeOption::Error
            ) =>
        {
            console_for_listener.fetch_add(1, Ordering::Relaxed);
        }
        Event::RuntimeExceptionThrown(_) => {
            exceptions_for_listener.fetch_add(1, Ordering::Relaxed);
        }
        Event::LogEntryAdded(entry)
            if matches!(entry.params.entry.level, Log::LogEntryLevel::Error) =>
        {
            log_for_listener.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }))
    .map_err(|_| BrowserError::Launch)?;
    Ok(PageObservers {
        http_status,
        console_errors,
        runtime_exceptions,
        log_errors,
    })
}

struct NetworkGuard {
    blocked: Arc<AtomicU32>,
    external: Arc<AtomicU32>,
}

fn attach_local_guard(
    tab: &headless_chrome::Tab,
    candidate: &Path,
) -> Result<NetworkGuard, BrowserError> {
    let candidate = candidate.to_path_buf();
    let blocked = Arc::new(AtomicU32::new(0));
    let blocked_for_interceptor = Arc::clone(&blocked);
    let blocked_external = Arc::new(AtomicU32::new(0));
    let external_for_interceptor = Arc::clone(&blocked_external);
    tab.enable_request_interception(Arc::new(
        move |_, _, event: Fetch::events::RequestPausedEvent| {
            if validate_local_target(&event.params.request.url, &candidate).is_ok() {
                RequestPausedDecision::Continue(None)
            } else {
                blocked_for_interceptor.fetch_add(1, Ordering::Relaxed);
                if Url::parse(&event.params.request.url)
                    .is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
                {
                    external_for_interceptor.fetch_add(1, Ordering::Relaxed);
                }
                RequestPausedDecision::Fail(Fetch::FailRequest {
                    request_id: event.params.request_id,
                    error_reason: Network::ErrorReason::BlockedByClient,
                })
            }
        },
    ))
    .map_err(|_| BrowserError::Launch)?;
    tab.enable_fetch(None, None)
        .map_err(|_| BrowserError::Launch)?;
    Ok(NetworkGuard {
        blocked,
        external: blocked_external,
    })
}

fn read_dom(tab: &headless_chrome::Tab, width: u32) -> Result<DomEvidence, BrowserError> {
    let script = RENDERED_DOM_SCRIPT.replace("__REQUESTED_WIDTH__", &width.to_string());
    let result = tab
        .evaluate(&script, false)
        .map_err(|_| BrowserError::Inspection)?;
    let serialized = result
        .value
        .and_then(|value| value.as_str().map(str::to_owned))
        .ok_or(BrowserError::Inspection)?;
    serde_json::from_str(&serialized).map_err(|_| BrowserError::Inspection)
}

fn save_screenshot(
    tab: &headless_chrome::Tab,
    screenshot_root: &Path,
    width: u32,
) -> Result<String, BrowserError> {
    let screenshot = tab
        .capture_screenshot(Page::CaptureScreenshotFormatOption::Png, None, None, true)
        .map_err(|_| BrowserError::Artifact)?;
    if screenshot.len() > MAX_SCREENSHOT_BYTES {
        return Err(BrowserError::Artifact);
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(screenshot_root.join(format!("viewport-{width}.png")))
        .map_err(|_| BrowserError::Artifact)?;
    file.write_all(&screenshot)
        .map_err(|_| BrowserError::Artifact)?;
    Ok(format!("browser/viewport-{width}.png"))
}

fn validate_local_target(target: &str, candidate: &Path) -> Result<(), BrowserError> {
    let url = Url::parse(target).map_err(|_| BrowserError::NonLocalTarget)?;
    if url.scheme() == "file" {
        let file = url
            .to_file_path()
            .map_err(|()| BrowserError::NonLocalTarget)?;
        let canonical = fs::canonicalize(&file).map_err(|_| BrowserError::NonLocalTarget)?;
        return if file == canonical && file.starts_with(candidate) && file.is_file() {
            Ok(())
        } else {
            Err(BrowserError::NonLocalTarget)
        };
    }
    let loopback = url.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if url.scheme() != "http" || !loopback || !url.username().is_empty() || url.password().is_some()
    {
        return Err(BrowserError::NonLocalTarget);
    }
    Ok(())
}

fn create_artifact_directory(root: &Path) -> Result<PathBuf, BrowserError> {
    let browser = root.join("browser");
    match fs::symlink_metadata(&browser) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(browser),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&browser).map_err(|_| BrowserError::Artifact)?;
            Ok(browser)
        }
        Ok(_) | Err(_) => Err(BrowserError::Artifact),
    }
}

const RENDERED_DOM_SCRIPT: &str = r#"JSON.stringify((() => {
  const requestedWidth = __REQUESTED_WIDTH__;
  const visible = (el) => {
    const style = getComputedStyle(el);
    const box = el.getBoundingClientRect();
    return style.display !== 'none' && style.visibility !== 'hidden' &&
      el.getAttribute('aria-hidden') !== 'true' && box.width > 0 && box.height > 0;
  };
  const label = (el) => {
    const labelled = (el.getAttribute('aria-labelledby') || '').split(/\s+/)
      .map(id => document.getElementById(id)?.textContent?.trim() || '').join(' ').trim();
    return (el.getAttribute('aria-label') || labelled ||
      el.labels?.[0]?.textContent || el.closest('label')?.textContent ||
      el.getAttribute('alt') || el.getAttribute('title') ||
      (el.tagName === 'INPUT' ? el.value : '') || el.textContent || '').trim();
  };
  const selector = (el) => {
    const id = el.id ? '#' + el.id : '';
    return el.tagName.toLowerCase() + id;
  };
  const nodes = Array.from(document.querySelectorAll('*'));
  const overflow = nodes.filter(el => visible(el) && el.getBoundingClientRect().right > requestedWidth + 1)
    .slice(0, 20).map(selector);
  const controls = Array.from(document.querySelectorAll('button,a[href],input:not([type=hidden]),select,textarea,[role=button],[tabindex]'));
  const unnamed = controls.filter(el => visible(el) && !el.disabled && !label(el))
    .slice(0, 20).map(selector);
  return {
    title: document.title.trim(),
    language: document.documentElement.lang.trim(),
    h1_count: document.querySelectorAll('h1').length,
    description: document.querySelector('meta[name="description"]')?.content?.trim() || null,
    canonical: document.querySelector('link[rel="canonical"]')?.href || null,
    robots: document.querySelector('meta[name="robots"]')?.content?.trim() || null,
    observed_width: innerWidth,
    document_width: document.documentElement.scrollWidth,
    overflow_elements: overflow,
    unnamed_controls: unnamed
  };
})())"#;
