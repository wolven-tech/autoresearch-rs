# Read-only market evidence imports

`autoresearch-market` reads operator-selected JSON sidecars. It never fetches
remote URIs, calls payment/provider APIs, sends outreach, or writes source
files. Imported receipts stay outside evaluator output and code-selection
policy. A receipt is provenance data, not automatic product-gate promotion.

Example sidecar:

```json
{
  "schema_version": 1,
  "receipt_type": "payment",
  "source": { "kind": "file", "path": "receipts/payment-001.pdf" },
  "occurred_at_unix_ms": 1789300000000,
  "evidence_sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
}
```

`receipt_type` must be `payment`, `fulfilment`, `refund`, `qualified_use`, or
`outreach`. `source` may instead be `{ "kind": "uri", "uri":
"https://example.com/receipt" }`; such URI is not fetched and digest stays
declared, not verified. Local file source must stay under evidence root and
match declared SHA-256. Sidecar path and source path are relative to root.
Credential-bearing URLs, queries, fragments, symlinks, oversized files,
missing timestamps, and missing or malformed digests are rejected.
