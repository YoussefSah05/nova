# Validating playlist imports

Import changes (`crates/xtream/`, `crates/m3u/`, `add_source` / `run_import` in the desktop
crate) are only done when a real provider playlist imports completely. The GUI can make a hang
look like "still loading", so use the headless probe.

## The probe

```bash
NOVA_PROBE_URL='<playlist url>' cargo run -p nova-xtream --example probe
```

It reproduces `add_source`'s routing and drains the import with a per-receive timeout, so a
stalled endpoint fails loudly. Pass the URL only through the environment variable: it carries
credentials and must never be committed. Add `RUST_LOG=debug` for endpoint detail.

| Result | Meaning |
|---|---|
| `OK: import completed cleanly` (exit 0) | Channel closed, entries imported, no errors. The only pass. |
| `FAIL: HANG` (exit 1) | No entry within the receive timeout: an endpoint stalled. The log names it (`get_live_streams`, `get_vod_streams` or `get_series`). |
| `FAIL: import reported N error(s)` | HTTP 4xx/5xx (credentials, unsupported endpoint) or a parse error (HTML or an object instead of an array) |
| `FAIL: … 0 entries` | Completed but empty: wrong URL type or an empty account |

## Loop

1. Make the change.
2. Run the probe until it reports `OK` with non-zero counts.
3. Confirm the same playlist once in the running app.

## Invariants

- The Xtream HTTP client keeps both a read timeout (stall detection) and a total timeout.
  Without them one stalled endpoint hangs the whole import, which only finishes when every
  endpoint's sender is dropped.
- `run_import` always emits a final `done` event, success or error.
- Errors from the final insert batch are reported through the progress event, like the others.
