# M05 / M16 — governed background work and job scheduling

Date: 2026-10-05. Status: **IN PROGRESS** — slice A implemented; slice B contracted.

Builds on the existing governor (`apps/desktop/src-tauri/src/resource_policy.rs`) and
the persisted per-profile queues (`desktop_jobs.rs`, `crates/jobs`). No new scheduler,
no schema change unless stated, no AI or frontend redesign.

## Slice A — route the remaining bypasses through the governor (M05)

The status matrix listed work that runs outside admission:

| Work | Before | After |
| --- | --- | --- |
| Series episode re-fetch after a refresh (`refetch_series_with_parked_history`) | Bare `tokio::spawn`, ran during playback | Metadata-class job slot. It waits while anything plays and checkpoints between series |
| Channel-logo matching (`match_channel_logos`) | Bare spawn, ran during playback | Metadata-class job slot, with a checkpoint before each source |
| M3U `url-tvg` registration | Detached spawn racing the guide refresh queued after import | Awaited after the import, before the caller queues the guide refresh |
| On-demand TMDB / Xtream fetches (title art, facts, series episodes on open) | Unadmitted | Unchanged by design: user-initiated interactive work, which the policy keeps available during playback |

Because a logo pass can now wait a long time (for example, through a whole film), the
review's findings were fixed:

- It checkpoints right after the directory download, not only before each source.
- The global "running" flag is replaced by a per-profile pending queue. A request
  made while a pass runs or waits is merged into it and served before the pass
  ends; it is no longer silently dropped.
- The setting is re-checked before every source, so turning the feature off stops
  a waiting pass.
- One source failing (for example, removed meanwhile) no longer aborts the others.
- A pass that ends abnormally (a panic in development builds, or tokio dropping
  its task) frees its profile's entry. Each pass owns its entry by id, so it can
  never remove a newer pass's.
- A batch queued after the feature was turned off takes no slot and downloads
  nothing.

No progress event is sent while a pass waits for playback to end. That is
acceptable, since the next event arrives when it runs.

The governor's existing tests cover the behaviour itself:

- background work is suspended during playback;
- checkpoints yield to playback and to higher-priority work;
- cancelled waiters leave no state.

This slice only moves the call sites onto it. No IPC, schema or UI change.

## Slice B — retries and scheduled refresh (M16)

Contract, not yet implemented:

- **Retry with backoff:** persisted jobs get retries with bounded exponential backoff,
  stored on the job row.
  - It applies only to failures classed as transient (network, provider timeouts).
  - Invalid payloads, a removed source or cancellation fail immediately.
  - Attempt count and the next eligible time are persisted, so retries survive
    restart.
  - Startup recovery keeps its current single requeue.
- **Scheduled refresh:** each source's `auto_refresh_hours` (stored today, never
  scheduled) enqueues a refresh when `last_refreshed_at + auto_refresh_hours` is due.
  - It runs as Metadata class, not Explicit, so it never competes with playback or
    with a user's own import.
  - Dedup is through the existing unique enqueue.
  - It is checked at startup and on a coarse timer. Sleep/wake follows the existing
    lifecycle hooks.
- **Tests:**
  - a transient failure retries, and a permanent one does not;
  - backoff bounds are respected and survive restart;
  - a due source enqueues exactly once;
  - scheduled refresh waits during playback.
- **Non-goals:** a general cron facility, per-provider rate limits, UI for retry
  history.
