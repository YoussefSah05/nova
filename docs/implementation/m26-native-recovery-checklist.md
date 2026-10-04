# M26 native recovery validation checklist

Purpose: exercise profile copy recovery and interruption cleanup in a running Tauri
app, using only synthetic data and an isolated app identity. This is the remaining
native evidence for M26 (see [`m26-profile-qualification.md`](m26-profile-qualification.md)).

## Isolation

| Concern | Isolated by |
| --- | --- |
| `app.db` and `profiles/` | `NOVA_VALIDATION_DIR` (debug builds only; must be absolute) |
| Artwork cache, models, WebKit storage, window state | Identifier `app.nova.desktop.validation` from `src-tauri/tauri.validation.conf.json` |
| Provider accounts | The fixture's only source is `example.invalid`; do not add a real source |

`just dev-validation` refuses relative paths: the app silently ignores a relative
`NOVA_VALIDATION_DIR` and would open the real data root. It also requires that the
directory already contains a synthetic `app.db`. Do not point either recipe at an
installed Nova data directory.

## Setup

```sh
DIR=/tmp/nova-recovery-validation-$(date +%Y%m%d-%H%M%S)
just recovery-fixture "$DIR"     # prints three synthetic profile IDs
just dev-validation "$DIR"       # window title: "Nova Validation (synthetic data)"
```

The fixture contains three profiles:

- **Validation legacy:** a known schema-21 lineage with 30,000 synthetic movies, so
  recovery takes about 9 s in a debug build.
- **Validation unknown lineage:** a tampered migration-6 checksum.
- **Validation healthy:** current schema.

Inspect the disk state between steps (read-only):

```sh
ls -la "$DIR/profiles"
sqlite3 -readonly "$DIR/app.db" "SELECT id, name FROM profiles ORDER BY created_at"
```

## Steps

Record pass/fail and any deviation for each.

1. **Isolation.**
   - The window title shows the validation name.
   - The picker lists exactly the three synthetic profiles.
   - No real profile appears.
2. **Healthy profile.**
   - It opens normally.
   - Return to the picker.
3. **Unsupported lineage.**
   - Opening "Validation unknown lineage" fails.
   - The recovery dialog says recovery is unsupported and offers no recovery action.
   - Focus returns to the picker when the dialog closes.
4. **Available recovery.**
   - Opening "Validation legacy" fails, and the dialog offers **Recover a copy**.
   - Start it. The pending state is shown, and a second submission is impossible.
   - On success, a "Validation legacy (Recovered)" profile appears. It is not
     selected automatically.
   - On disk: one new UUID directory, no `.nova-recovery-v1-*` directory, and no
     `.nova-recovery-v1.json` inside the new directory.
   - The original directory's `profile.db` checksum is unchanged. Record
     `shasum -a 256` before step 4.
5. **Open the recovered copy.**
   - It opens; Movies lists synthetic films.
   - Favorites and watchlist entries (every 50th and 70th film) are present.
   - Continue Watching shows the films with saved progress.
   - Quit and relaunch; it opens again.
6. **Quit during recovery.**
   - Delete the recovered profile in the app. Its directory stays on disk by design.
   - Start a new recovery of "Validation legacy". Within about 5 s, force-quit the
     app (Activity Monitor → Force Quit, or `kill -9` on the Nova Validation process).
   - On disk: a `.nova-recovery-v1-<uuid>-*` staging directory may remain. Expect no
     new registered profile.
7. **Retry after interruption.**
   - Relaunch with `just dev-validation "$DIR"` and recover again. It succeeds.
   - On disk: the interrupted staging directory is gone, and any unregistered copy it
     published is gone.
   - The profile deleted in step 6 (which had been opened) still exists on disk.
   - Exactly one new recovered profile is registered.
8. **Cancellation from the UI side.**
   - Start a recovery, then close the dialog (or reload with Cmd+R) while it is
     pending.
   - Recovery either completes and registers a copy, or leaves only receipts that the
     next retry cleans up.
   - The profile picker stays usable. Opening a profile waits for recovery rather
     than racing it.
9. **Original preservation.**
   - Re-check the legacy `profile.db` checksum from step 4.
   - Opening the original still fails and still offers recovery.

## Afterwards

Quit the validation app. Then remove the synthetic data and whichever of the
isolated identity's directories exist:

- `$DIR`
- `~/Library/Application Support/app.nova.desktop.validation`
- `~/Library/Caches/app.nova.desktop.validation`
- `~/Library/WebKit/app.nova.desktop.validation`

Record the results in the M26 qualification document and the status file. Do not
commit screenshots that show anything outside the validation window.
