//! Declarative portfolio templates freeze every relevant local policy input.

use autoresearch_config::{FrozenIdentity, ValidatedManifest};
use std::collections::BTreeMap;

const UI: &str = include_str!("../../../examples/portfolio/ui/autoresearch.toml");
const COPY: &str = include_str!("../../../examples/portfolio/copy/autoresearch.toml");
const PERFORMANCE: &str = include_str!("../../../examples/portfolio/performance/autoresearch.toml");
const UI_PROGRAM: &str = include_str!("../../../examples/portfolio/ui/program.md");
const PORTFOLIO_README: &str = include_str!("../../../examples/portfolio/README.md");
const SEO: &str = include_str!("../../../examples/portfolio/seo/autoresearch.toml");
const GEO: &str = include_str!("../../../examples/portfolio/geo/autoresearch.toml");

#[test]
fn ui_copy_and_performance_templates_are_bounded_and_local() {
    for forbidden_promotion_signal in [
        "Traffic",
        "clicks",
        "impressions",
        "praise",
        "synthetic telemetry",
    ] {
        assert!(PORTFOLIO_README.contains(forbidden_promotion_signal));
    }
    for source in [UI, COPY, PERFORMANCE] {
        let manifest = ValidatedManifest::parse(source).expect("portfolio template parses");
        let web = manifest.web().expect("local web targets");
        assert_eq!(web.origin(), "http://127.0.0.1:4402");
        assert_eq!(web.viewports(), [320, 390, 768, 1280]);
        assert!(web.reduced_motion());
        assert_eq!(web.routes().len(), 1);
        assert!(manifest.experiment().budget().max_candidates <= 2);
        assert!(manifest.experiment().budget().wall_clock_seconds <= 180);
        assert!(manifest.authority().allowed().is_empty());
        assert_eq!(manifest.evaluators().len(), 2);
    }
}

#[test]
fn frozen_identity_changes_with_threshold_route_prompt_and_adapter_version() {
    let original = ValidatedManifest::parse(UI).expect("UI manifest");
    let fixtures = BTreeMap::from([("route-home".to_owned(), b"source fixture".to_vec())]);
    let baseline =
        FrozenIdentity::capture(&original, UI_PROGRAM.as_bytes(), &fixtures, Some(b"gate"))
            .expect("frozen identity");
    for changed in [
        UI.replace("max_lcp_ms = 2500", "max_lcp_ms = 2400"),
        UI.replace("path = \"/\"", "path = \"/guide\""),
        UI.replace("ui_browser_v1", "ui_browser_v2"),
        UI.replace("--adapter-version\", \"1", "--adapter-version\", \"2"),
    ] {
        let parsed = ValidatedManifest::parse(&changed).expect("changed valid manifest");
        let identity =
            FrozenIdentity::capture(&parsed, UI_PROGRAM.as_bytes(), &fixtures, Some(b"gate"))
                .expect("changed identity");
        assert_ne!(baseline.aggregate_sha256, identity.aggregate_sha256);
    }
    let changed_prompt =
        FrozenIdentity::capture(&original, b"different prompt", &fixtures, Some(b"gate"))
            .expect("prompt identity");
    assert_ne!(baseline.aggregate_sha256, changed_prompt.aggregate_sha256);
    let changed_fixture = FrozenIdentity::capture(
        &original,
        UI_PROGRAM.as_bytes(),
        &BTreeMap::from([("route-home".to_owned(), b"different source".to_vec())]),
        Some(b"gate"),
    )
    .expect("fixture identity");
    assert_ne!(baseline.aggregate_sha256, changed_fixture.aggregate_sha256);
}

#[test]
fn seo_geo_templates_bind_source_policy_and_keep_network_disabled() {
    let seo = ValidatedManifest::parse(SEO).expect("SEO template");
    let geo = ValidatedManifest::parse(GEO).expect("GEO template");
    for manifest in [&seo, &geo] {
        let web = manifest.web().expect("web policy");
        let policy = web.seo().expect("technical SEO policy");
        assert_eq!(policy.robots_path(), "/robots.txt");
        assert_eq!(policy.sitemap_path(), "/sitemap.xml");
        let production = web.production().expect("explicit read-only allowlist");
        assert_eq!(production.origins(), ["https://example.com"]);
        assert!(manifest.authority().allowed().is_empty());
        assert!(manifest.experiment().budget().max_candidates <= 2);
    }
    assert_eq!(seo.web().expect("web").routes().len(), 1);
    let geo_web = geo.web().expect("geo web");
    assert_eq!(geo_web.routes().len(), 2);
    assert_eq!(
        geo_web.geo().expect("source policy").source_urls(),
        Vec::<String>::new()
    );
    assert!(
        ValidatedManifest::parse(&GEO.replace(
            "source_urls = []",
            "source_urls = [\"https://synthetic.example.invalid/fake?query=1\"]"
        ))
        .is_err()
    );
    let declared = GEO.replace(
        "source_urls = []",
        "source_urls = [\"https://example.com/source\"]",
    );
    let declared_manifest = ValidatedManifest::parse(&declared).expect("exact HTTPS source");
    assert_eq!(
        declared_manifest
            .web()
            .expect("web")
            .geo()
            .expect("geo")
            .source_urls(),
        ["https://example.com/source"]
    );
}
