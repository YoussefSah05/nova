# Performance engineering

Nova is built and measured against a real provider playlist of about 42,000 entries (8,658 live
channels, ~28,000 films, ~4,800 shows) on an Apple Silicon MacBook Pro with a 120 Hz display.
This page records the budgets, the tools, and what measurement has taught us.

## Budgets

| Area | Target |
|---|---|
| Interface frame | 8.3 ms at 120 Hz (16.7 ms at 60 Hz), including React work, style, layout, paint |
| Catalog query behind a screen | under 30 ms on the real playlist |
| Channel switch (resolve a live variant) | under 5 ms |
| Playlist import (42k entries) | a few seconds, with enrichment in the background |
| Background AI work | never while something plays (resource governor) |

## Tools

### Frame-rate overlay

Press **⌘⌥F** in the app. It shows the live frame rate and the measured display rate, and records
each scroll gesture per screen: average FPS, 1% low, 95th-percentile and worst frame time, and
janky frames (longer than 1.5 refresh intervals). **Copy report** produces a text summary.
Measure release builds (`just build`); development builds run unoptimized React.

### Live query probe

```bash
cp ~/Library/Application\ Support/app.nova.desktop/profiles/<id>/profile.db /tmp/copy.db
NOVA_LIVE_PERF_DB=/tmp/copy.db cargo run --release -p nova-storage --example live_perf
```

Times every query behind the Live screen on a copy of a real profile (never the original: the
probe recompiles the live projection).

### SQLite query plans

`EXPLAIN QUERY PLAN` in `sqlite3` on a profile copy shows whether a query walks an index or
re-runs work per row. The `.sqlx/` offline metadata keeps every query compile-time checked.

## Case studies

### Live: a quadratic CTE (73 s to 2 ms)

Opening Live showed a skeleton for about a minute. The query that selects a page of channel keys
was a common table expression that SQLite ran as a co-routine on the inner side of a join, so it
re-sorted all 8,658 channels by JSON fields once per channel row (about 75 million JSON reads per
page). Materializing the CTE, driving the join from it through the `(source_id,
live_channel_key)` index, and sorting on indexed columns instead of JSON brought a guide page to
1.9 ms and a channel switch from 5.5 s to 0.4 ms. A 4,000-channel regression test keeps it that
way.

### Shows: CSS filters on every card

At 60 Hz, Shows ran at 33–45 fps while Movies held 60. Each show card set its title with
Tailwind's `drop-shadow`, two CSS filters that WebKit renders offscreen for every card as rows
scroll in. An equivalent `text-shadow` costs almost nothing. Correct row-height estimates for the
virtualized grid stopped the virtualizer from correcting rows mid-scroll.

### 120 Hz: lifting WebKit's cap

WebKit paces page rendering near 60 fps by default, even on ProMotion displays, and Tauri apps
inherit that. Nova turns off WebKit's `PreferPageRenderingUpdatesNear60FPSEnabled` feature on its
webview (Settings → Appearance → High refresh rate). Measured: Home 117 fps at ~125 Hz.

### Per-card and per-logo work while scrolling

At 120 Hz the frame budget halves, and work that fit at 60 Hz shows up as dropped frames:

- a Radix context menu per card cost about 66 µs to mount; grids now share one menu;
- each new channel logo was drawn into a canvas to pick its tile colour, forcing a synchronous
  image decode mid-scroll; sampling now decodes asynchronously, waits until scrolling stops,
  and is persisted across launches;
- `loading="lazy"` delayed image loads until rows were almost visible; virtualized rows now load
  eagerly, so overscan rows decode before they appear.

## Rules of thumb

1. **Measure on the real library, in a release build.** Synthetic data hides quadratic queries;
   development builds hide nothing useful.
2. **Nothing optional runs while content moves.** Use `whenScrollIdle()` and the `nova-scrolling`
   class; hover effects, previews and prefetch are already suppressed during scroll.
3. **No CSS `filter`, `backdrop-filter` or large blurred shadows on elements that repeat in a
   grid.** Use solid colours, `text-shadow`, or a single surface.
4. **Never draw an undecoded image into a canvas.** `await img.decode()` first.
5. **Virtualize long lists**, give the virtualizer accurate size estimates, and keep cards cheap
   to mount (no per-card portals, menus or providers).
6. **Check query plans for anything that touches every channel or title**, and prefer indexed
   columns over `json_extract` in `ORDER BY` and `JOIN`.
7. **Avoid `will-change` and 3D transforms on many elements**: they create compositing layers and
   prevent WebKit's asynchronous image decoding.

## Further reading

- [WebKit bug 290671](https://bugs.webkit.org/show_bug.cgi?id=290671): rendering cadence on
  ProMotion displays
- [WebKit changeset 213764](https://trac.webkit.org/r213764): asynchronous decoding of large images
- [TanStack Virtual performance work](https://tanstack.com/blog/tanstack-virtual-perf-and-ios)
- [content-visibility](https://web.dev/articles/content-visibility) on web.dev
