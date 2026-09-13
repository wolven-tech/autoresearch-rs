//! Explicit-authority, read-only HTTPS production probes.

use autoresearch_config::{ExternalCapability, ProductionSettings, ValidatedManifest};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;
use url::Url;

/// Production probe request; only exact GET and HEAD are accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductionProbeRequest {
    /// Exact HTTPS URL within frozen origin/path allowlist.
    pub url: String,
    /// HTTP method, exactly `GET` or `HEAD`.
    pub method: String,
}

/// Explicit per-run network permission. Default denies all network access.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NetworkReadPermission {
    granted: bool,
}

impl NetworkReadPermission {
    /// Records caller's explicit per-run read-only network authorization.
    #[must_use]
    pub const fn granted() -> Self {
        Self { granted: true }
    }
}

/// Probe safety or allowlist failure; no request is sent after failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProductionProbeError {
    /// Method can mutate remote state or is unsupported.
    #[error("production probe permits only GET and HEAD")]
    MutationMethod,
    /// URL is not exact allowlisted HTTPS without credentials/query/fragment.
    #[error("production probe URL is not exact allowlisted HTTPS")]
    DisallowedUrl,
    /// Redirect target leaves exact frozen allowlist.
    #[error("production redirect target is not allowlisted")]
    DisallowedRedirect,
    /// Redirect chain is incomplete or exceeds frozen bound.
    #[error("production redirect chain exceeds frozen bound or omits location")]
    RedirectLimit,
}

/// Availability is distinct from a passing technical or product gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeAvailability {
    /// A real read-only response was received.
    Available,
    /// Missing permission, policy, or network; never counts as pass.
    Unavailable,
}

/// One observed read-only HTTPS response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionHop {
    /// Exact checked URL.
    pub url: String,
    /// Observed status.
    pub status: u16,
}

/// Bounded response metadata. No deployment, form submission, or write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionEvidence {
    /// Availability, never a gate pass flag.
    pub availability: ProbeAvailability,
    /// Observed responses when available.
    pub hops: Vec<ProductionHop>,
    /// Final URL when available.
    pub final_url: Option<String>,
    /// Stable reason when unavailable.
    pub unavailable_reason: Option<String>,
}

impl ProductionEvidence {
    fn unavailable(reason: &str) -> Self {
        Self {
            availability: ProbeAvailability::Unavailable,
            hops: Vec::new(),
            final_url: None,
            unavailable_reason: Some(reason.into()),
        }
    }
}

#[derive(Clone, Copy)]
enum ReadMethod {
    Get,
    Head,
}

impl ReadMethod {
    fn parse(value: &str) -> Result<Self, ProductionProbeError> {
        match value {
            "GET" => Ok(Self::Get),
            "HEAD" => Ok(Self::Head),
            _ => Err(ProductionProbeError::MutationMethod),
        }
    }
}

struct TransportResponse {
    status: u16,
    location: Option<String>,
}

trait ReadTransport {
    fn send(&self, url: &Url, method: ReadMethod) -> Result<TransportResponse, ()>;
}

struct UreqTransport;

impl ReadTransport for UreqTransport {
    fn send(&self, url: &Url, method: ReadMethod) -> Result<TransportResponse, ()> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .into();
        let response = match method {
            ReadMethod::Get => agent.get(url.as_str()).call(),
            ReadMethod::Head => agent.head(url.as_str()).call(),
        }
        .map_err(|_| ())?;
        Ok(TransportResponse {
            status: response.status().as_u16(),
            location: response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        })
    }
}

/// Probes frozen HTTPS URL only when manifest permits network and caller grants it.
///
/// This performs only bounded GET/HEAD requests. No live probe is made by
/// default; a policy declaration alone never grants authority. Transport
/// failure yields unavailable evidence, never a passing gate.
///
/// # Errors
///
/// Rejects mutation methods, unsafe URLs, and off-allowlist redirects before
/// making any subsequent request.
pub fn probe_production(
    manifest: &ValidatedManifest,
    request: &ProductionProbeRequest,
    permission: NetworkReadPermission,
) -> Result<ProductionEvidence, ProductionProbeError> {
    probe_with_transport(manifest, request, permission, &UreqTransport)
}

fn probe_with_transport(
    manifest: &ValidatedManifest,
    request: &ProductionProbeRequest,
    permission: NetworkReadPermission,
    transport: &impl ReadTransport,
) -> Result<ProductionEvidence, ProductionProbeError> {
    let method = ReadMethod::parse(&request.method)?;
    let Some(policy) = manifest.web().and_then(|web| web.production()) else {
        return Ok(ProductionEvidence::unavailable("disabled_by_manifest"));
    };
    let mut current = Url::parse(&request.url).map_err(|_| ProductionProbeError::DisallowedUrl)?;
    if !allowed_url(policy, &current) {
        return Err(ProductionProbeError::DisallowedUrl);
    }
    if !manifest.authority().permits(ExternalCapability::Network) || !permission.granted {
        return Ok(ProductionEvidence::unavailable(
            "network_permission_missing",
        ));
    }
    let mut hops = Vec::new();
    loop {
        let Ok(response) = transport.send(&current, method) else {
            return Ok(ProductionEvidence::unavailable("network_unavailable"));
        };
        hops.push(ProductionHop {
            url: current.to_string(),
            status: response.status,
        });
        if (300..400).contains(&response.status) {
            if hops.len() > usize::from(policy.max_redirects()) {
                return Err(ProductionProbeError::RedirectLimit);
            }
            let location = response
                .location
                .ok_or(ProductionProbeError::RedirectLimit)?;
            let next = current
                .join(&location)
                .map_err(|_| ProductionProbeError::DisallowedRedirect)?;
            if !allowed_url(policy, &next) {
                return Err(ProductionProbeError::DisallowedRedirect);
            }
            current = next;
            continue;
        }
        return Ok(ProductionEvidence {
            availability: ProbeAvailability::Available,
            hops,
            final_url: Some(current.to_string()),
            unavailable_reason: None,
        });
    }
}

fn allowed_url(policy: &ProductionSettings, url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && policy
            .origins()
            .iter()
            .any(|origin| origin == &url.origin().ascii_serialization())
        && policy.paths().iter().any(|path| path == url.path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    const BASE: &str = r#"
schema_version = 1
[experiment]
name = "production fixture"
[experiment.objective]
name = "score"
direction = "maximize"
[experiment.budget]
max_candidates = 1
max_failures = 1
wall_clock_seconds = 30
[scope]
mutable_paths = ["src"]
[agent]
program = "manual"
timeout_seconds = 10
[[evaluators]]
id = "fixture"
[evaluators.command]
program = "true"
timeout_seconds = 10
[[evaluators.metrics]]
name = "score"
kind = "objective"
direction = "maximize"
[web]
origin = "http://127.0.0.1:4402"
viewports = [320, 390, 768, 1280]
[[web.routes]]
name = "home"
path = "/"
expected_status = 200
[web.production]
origins = ["https://fixture.example", "https://other.example"]
paths = ["/", "/ready"]
max_redirects = 2
[authority]
allow = ["network"]
"#;

    struct FakeTransport {
        calls: Cell<u32>,
        replies: RefCell<Vec<Result<TransportResponse, ()>>>,
    }
    impl FakeTransport {
        fn new(replies: Vec<Result<TransportResponse, ()>>) -> Self {
            Self {
                calls: Cell::new(0),
                replies: RefCell::new(replies.into_iter().rev().collect()),
            }
        }
    }
    impl ReadTransport for FakeTransport {
        fn send(&self, _: &Url, _: ReadMethod) -> Result<TransportResponse, ()> {
            self.calls.set(self.calls.get() + 1);
            self.replies.borrow_mut().pop().expect("fake reply")
        }
    }
    fn manifest() -> ValidatedManifest {
        ValidatedManifest::parse(BASE).expect("fixture policy")
    }
    fn request(url: &str, method: &str) -> ProductionProbeRequest {
        ProductionProbeRequest {
            url: url.into(),
            method: method.into(),
        }
    }
    fn response(status: u16, location: Option<&str>) -> TransportResponse {
        TransportResponse {
            status,
            location: location.map(str::to_owned),
        }
    }

    #[test]
    fn denied_by_default_and_network_unavailable_never_pass() {
        let fake = FakeTransport::new(vec![]);
        let denied = probe_with_transport(
            &manifest(),
            &request("https://fixture.example/", "GET"),
            NetworkReadPermission::default(),
            &fake,
        )
        .expect("denied");
        assert_eq!(denied.availability, ProbeAvailability::Unavailable);
        assert!(denied.hops.is_empty());
        assert_eq!(fake.calls.get(), 0);
        let no_ceiling =
            ValidatedManifest::parse(&BASE.replace("allow = [\"network\"]", "allow = []"))
                .expect("manifest without network ceiling");
        let ceiling_denied = probe_with_transport(
            &no_ceiling,
            &request("https://fixture.example/", "GET"),
            NetworkReadPermission::granted(),
            &fake,
        )
        .expect("ceiling denied");
        assert_eq!(ceiling_denied.availability, ProbeAvailability::Unavailable);
        assert_eq!(fake.calls.get(), 0);
        let no_policy = ValidatedManifest::parse(&BASE.replace("[web.production]\norigins = [\"https://fixture.example\", \"https://other.example\"]\npaths = [\"/\", \"/ready\"]\nmax_redirects = 2\n", ""))
            .expect("manifest without production policy");
        let policy_denied = probe_with_transport(
            &no_policy,
            &request("https://fixture.example/", "GET"),
            NetworkReadPermission::granted(),
            &fake,
        )
        .expect("policy denied");
        assert_eq!(policy_denied.availability, ProbeAvailability::Unavailable);
        assert_eq!(fake.calls.get(), 0);
        let unavailable = probe_with_transport(
            &manifest(),
            &request("https://fixture.example/", "HEAD"),
            NetworkReadPermission::granted(),
            &FakeTransport::new(vec![Err(())]),
        )
        .expect("network unavailable");
        assert_eq!(unavailable.availability, ProbeAvailability::Unavailable);
        assert!(unavailable.final_url.is_none());
    }

    #[test]
    fn rejects_mutation_non_https_credentials_and_unlisted_path_before_send() {
        let fake = FakeTransport::new(vec![]);
        for (url, method, expected) in [
            (
                "https://fixture.example/",
                "POST",
                ProductionProbeError::MutationMethod,
            ),
            (
                "http://fixture.example/",
                "GET",
                ProductionProbeError::DisallowedUrl,
            ),
            (
                "https://user:pass@fixture.example/",
                "GET",
                ProductionProbeError::DisallowedUrl,
            ),
            (
                "https://fixture.example/private",
                "GET",
                ProductionProbeError::DisallowedUrl,
            ),
            (
                "https://fixture.example/?write=1",
                "GET",
                ProductionProbeError::DisallowedUrl,
            ),
        ] {
            let error = probe_with_transport(
                &manifest(),
                &request(url, method),
                NetworkReadPermission::granted(),
                &fake,
            )
            .expect_err("unsafe request");
            assert_eq!(error, expected);
        }
        assert_eq!(fake.calls.get(), 0);
    }

    #[test]
    fn bounded_redirect_rechecks_origin_and_path_before_second_send() {
        let fake = FakeTransport::new(vec![Ok(response(302, Some("https://evil.example/")))]);
        let error = probe_with_transport(
            &manifest(),
            &request("https://fixture.example/", "GET"),
            NetworkReadPermission::granted(),
            &fake,
        )
        .expect_err("unsafe redirect");
        assert_eq!(error, ProductionProbeError::DisallowedRedirect);
        assert_eq!(fake.calls.get(), 1);
        let allowed = FakeTransport::new(vec![
            Ok(response(302, Some("/ready"))),
            Ok(response(200, None)),
        ]);
        let evidence = probe_with_transport(
            &manifest(),
            &request("https://fixture.example/", "HEAD"),
            NetworkReadPermission::granted(),
            &allowed,
        )
        .expect("allowed redirect");
        assert_eq!(evidence.availability, ProbeAvailability::Available);
        assert_eq!(evidence.hops.len(), 2);
        assert_eq!(allowed.calls.get(), 2);
    }
}
