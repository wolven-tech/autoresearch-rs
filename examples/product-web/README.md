# Product-web fixture

Three local routes: `/` (normal), `/metadata` (metadata), and `/missing`
(expected 404). Pages have no external dependencies. Run owned Rust fixture
server with `cargo run -p autoresearch-product-web --example product_web_fixture`.
It binds only `127.0.0.1:4419`; stop with Ctrl-C. Frozen route/viewport settings
live in `autoresearch.toml`. Lab checks here cannot establish product demand or
change any product bet gate.
