# UI-6: Library and history

Reuse the existing paged BrowseGrid, saved-item actions, navigation, artwork,
playback and source cache. Replace the Library rails with Watchlist, Favourites
and History views. Saved views filter films, shows and channels and support
search. Films and shows support title/playlist-added order; channels use the
existing family grouping (the backend does not expose alternative orders).

Add a read-only, profile- and source-scoped `list_library_history` command over
durable movie and episode progress and current live-channel history. Return the
latest watch per film, episode or channel, including completed and unavailable
items. Group by local date; page 50 rows with a one-row lookahead. Playback uses
current media IDs, with explicit replay from zero for completed items.

No migration, progress writes, deletion, playback-event logging, AI requests,
new provider access, or UI-7 work. Hidden screens unsubscribe; source changes
reset local controls. Catalog refresh and player close invalidate history.
Player teardown refreshes again after its final asynchronous progress write.
Saved membership changes refresh filtered grids/counts. The new IPC DTO is
generated from Rust using ts-rs (`just bindings`); existing legacy DTOs remain
out of scope. No production dependency is needed for type generation.

Known persistence limit: absent episodes have no retained episode title or
numbering kind. Saved coordinates can be synthetic, so absent episodes show
the series name and an honest “Episode” label with date/progress. Do not infer
season numbering from those coordinates. Live history requires a current
channel row and does not retain vanished channels.

Validate storage scope, identity, unavailable/completed items, deterministic
paging, UI filters/empty/error states and playback actions. Run relevant broad
checks, isolated native smoke, and an independent milestone review.
