# M05 / M16 — governed background work and job scheduling

Date: 2026-10-05. Status: **IN PROGRESS** — slice A done; B1 (scheduled refresh, opt-in) implemented; B2 (retry/backoff) contracted.

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

## Slice B1 — scheduled playlist refresh (M16), implemented

Settings already offered a per-playlist auto-refresh interval: Manual, 6 h, 12 h,
daily or weekly. It was stored, defaulting to 24 hours, but never acted on.

**Decision (user, 2026-10-05):** automatic refresh is opt-in.

- Migration 0043 sets every playlist to Manual. No playlist had ever
  auto-refreshed, and a deliberate "Daily" cannot be told apart from the old
  default.
- New playlists are created on Manual.
- `set_source_auto_refresh` accepts only Manual or the offered intervals.
- The Settings description says refresh happens while Nova is open and pauses
  during playback.

The scheduler:

- **Due rule:** a source is due when `last_refreshed_at + hours * 3600 <= now`.
  Hours `0` is manual. A source that never imported successfully is left to the
  user.
- **When it is checked:** when a profile is selected, then every 15 minutes, and
  only while that profile is the active one. Each source is enqueued at most once
  per interval even if its refresh keeps failing. That memory is in-process, so a
  restart allows one more attempt.
- **Queue:** scheduled refreshes have their own queue (`desktop_scheduled_refresh`)
  with metadata admission. They wait while anything plays and never hold up a
  user's import, which keeps its own explicit queue.
- **One import per source:** a per-source lock shared by both queues keeps one
  import per source at a time. Under that lock the scheduled job re-checks the due
  rule, so a refresh that another refresh overtook does nothing.
- **Guide refresh:** the follow-up guide refresh is queued at the import's own
  priority, so a background refresh's guide download stays background work.
- **Progress:** progress goes to `playlist:import:background`. On success the UI
  quietly invalidates the active source's caches and starts enrichment and logo
  matching. It never shows a background refresh's progress or errors.
- **Bounded:** an import that is already running continues if playback starts. It
  is one atomic transaction, as with a manual refresh.

Independent review of the first version found three problems, all fixed by this
design:

- a user import could be parked behind a waiting background job for a whole film;
- background progress reset the UI and raised errors;
- background work escalated its guide refresh to explicit priority.

It also found that profiles the user had left kept refreshing; scheduling is now
limited to the active profile.

A re-review confirmed those fixes and found no deadlock. Since source ids are per
profile, a background refresh that had already started for a profile the user left
could match a source of the new profile. Background events now carry `profileId`,
and the UI ignores other profiles' events.

Accepted, low:

- A background refresh of a non-active playlist does not invalidate Settings'
  "last refreshed" display until its next load.
- Per-source lock entries are never removed (one small entry per source).

## Slice B2 — retries with backoff (M16)

Contract, not yet implemented:

- **Retry with backoff:** persisted jobs get retries with bounded exponential backoff,
  stored on the job row.
  - It applies only to failures classed as transient (network, provider timeouts).
  - Invalid payloads, a removed source or cancellation fail immediately.
  - Attempt count and the next eligible time are persisted, so retries survive
    restart.
  - Startup recovery keeps its current single requeue.
- **Tests:**
  - a transient failure retries, and a permanent one does not;
  - backoff bounds are respected and survive restart;
  - a due source enqueues exactly once;
  - scheduled refresh waits during playback.
- **Non-goals:** a general cron facility, per-provider rate limits, UI for retry
  history.
