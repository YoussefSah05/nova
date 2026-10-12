## What and why

<!-- What does this change do, and what problem does it solve? Link the issue or milestone. -->

## How it was validated

- [ ] `just lint && just typecheck && just test`
- [ ] New or changed behaviour has tests
- [ ] Manually checked in a release build (`just build`) when the change is visible or performance-related
<!-- Include before/after numbers for performance work (frame-rate overlay, live_perf probe). -->

## Checklist

- [ ] SQL changes: `.sqlx/` offline metadata updated; migrations are additive and numbered
- [ ] IPC changes: documented in `docs/ipc-contract.md`; types regenerated with `just bindings`
- [ ] No provider URLs, credentials or file paths reach the webview or logs
- [ ] Docs updated (`docs/nova-implementation-status.md`, user-facing guides, CHANGELOG)
