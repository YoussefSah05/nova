# Changelog

## 0.2.0-alpha.0 — in development

- Add a development stack manifest and read-only stack/environment verification commands.
- Pin selected Rust/Node/pnpm/just versions; use locked package resolution and explicit bootstrap.
- Add environment drift regression tests and CI manifest validation.
- Catalog and player runtime behavior remain on the existing implementation.
- Add unconsumed catalog source contracts with typed IDs, bounded preserved evidence,
  explicit unknown numbering and diagnostic redaction; no profile migration yet.

## 0.1.0 — pre-migration historical baseline

- Preserve the existing Nova application and architecture proposals before production hardening.
- Record the existing Cargo dependency resolution and available validation evidence.
- Existing capabilities include provider import, browsing, native playback, local captions,
  translation and search; this checkpoint is not a publicly certified release.
- Known validation gaps: pre-existing Rust formatting differences, no actual-app E2E,
  no clean packaged-app/device certification, and unverified remote CI/protection.
- See `docs/implementation/baseline.md` for environment caveats and evidence.
