# UI-7: Settings, models and storage

Reuse the settings overlay entry point, profile preference writer, caption controls,
model/download/removal commands, runtime status, external-library authorization,
cache commands, playlist/guide management and write-only TMDB credential flow.

Replace the old overlay with an accessible dialog and labelled section navigation:
Playback & captions, Appearance, Models, Storage, Catalog, Playlists & guide, and
Profile & about. Keep Nova's existing typography, surfaces, accent and reduced-motion
system. Restore focus on close, contain keyboard focus, support Escape, and keep
portal-based selects usable. Prevent Command-K from stacking over Settings.

Unify Whisper and embedding installation in Models; separate cache management into
Storage. Display real loading/error/retry states, installed/unavailable model status,
per-artifact Whisper progress, external libraries and a compact runtime workload
summary. In-flight model mutations survive leaving/reopening their view; no fake
cancel, progress percentages or inference routing. Removal/clearing retain explicit
confirmation. Improve action errors and empty states in playlist/profile management.
Catalog keeps opt-in metadata configuration and manual enrichment; model installation
links to Models. Preserve masked/write-only credentials and profile-scoped writes.

No migrations, new model backend, automatic downloads, model selection policy,
external-model execution, scheduler changes, network permission changes or later
milestone work. Existing IPC DTOs used by the new model/storage data layer will be
exported from Rust using the established test-only ts-rs pattern.

Validate dialog keyboard/focus behavior, navigation, loading/retry/error states,
model progress and mutations across remounts, scoped cache actions and existing
credential/persistence regressions. Run broader checks, native smoke using isolated
synthetic data without downloading models/deleting user data, and independent review.
