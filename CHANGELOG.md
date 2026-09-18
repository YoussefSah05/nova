# Changelog

## 0.2.0-alpha.0 — in development

- Compare pinned Qwen and Ministral models on development and frozen synthetic
  transfer cases; choose Qwen 4B/v2 as a development reference, not a serving-app model.
- Add a development catalog-model comparison plan, synthetic multilingual smoke
  corpus and bounded local inference evaluation harness with CI regression tests.
- Establish required catalog intelligence in the plan and add an unconnected,
  strict proposal/evidence evaluator with conflict, expiry and authority checks.
- Add a development stack manifest and read-only stack/environment verification commands.
- Pin selected Rust/Node/pnpm/just versions; use locked package resolution and explicit bootstrap.
- Add environment drift regression tests and CI manifest validation.
- Catalog and player runtime behavior remain on the existing implementation.
- Add unconsumed catalog source contracts with typed IDs, bounded preserved evidence,
  explicit unknown numbering and diagnostic redaction; no profile migration yet.
- Add bounded snapshot capture, immutable completion receipts and safe per-endpoint
  partial-refresh selection, plus versioned SHA-256 metadata/locator fingerprints.
- Add a synthetic loopback IPTV provider and real-client integration tests for M3U,
  Xtream, lazy episodes and XMLTV.
- Correct M3U episode suffix offsets for Unicode titles; regression tests pass,
  with real-provider and running-app validation still outstanding.
- Add an unconnected encrypted snapshot file-store prototype with authenticated
  replay, private files, no-overwrite commits and process-interruption tests.
- Add profile snapshot-key provision/load contracts and a macOS Keychain adapter;
  native provisioning still requires signing-entitlement validation.
- Add v2 streaming encrypted snapshots with authenticated final receipts, bounded
  queues and conservative byte/file quotas; automatic eviction remains pending.
- Add an unconnected Xtream object adapter and encrypted record archives that
  preserve raw evidence, duplicate upstream occurrences and bounded source fields.

## 0.1.0 — pre-migration historical baseline

- Preserve the existing Nova application and architecture proposals before production hardening.
- Record the existing Cargo dependency resolution and available validation evidence.
- Existing capabilities include provider import, browsing, native playback, local captions,
  translation and search; this checkpoint is not a publicly certified release.
- Known validation gaps: pre-existing Rust formatting differences, no actual-app E2E,
  no clean packaged-app/device certification, and unverified remote CI/protection.
- See `docs/implementation/baseline.md` for environment caveats and evidence.
