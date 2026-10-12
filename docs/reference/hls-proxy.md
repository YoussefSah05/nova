# Stream proxy

`crates/hls-proxy/` runs a loopback Axum server that every stream passes through. It exists so
the player never sees a provider URL and so manifests can be rewritten in one place.

## Routes

Every route is scoped to an opaque, per-session token; nothing a session's manifests did not name
is ever fetched.

| Route | Serves |
|---|---|
| `/stream/{token}/master.m3u8` | The session's manifest, with every URI rewritten to `/stream/{token}/r/{id}` |
| `/stream/{token}/r/{id}` | A segment, key or sub-playlist the session's manifests named (sub-playlists are rewritten the same way) |
| `/stream/{token}/file` | A direct-file session's bytes (Range requests forwarded) |
| `/stream/{token}/local/{file}` | Files FFmpeg writes for a remux |
| `/captions/{token}/…` | Reserved for caption tracks (returns 404 today) |

## Manifest rules

- A master playlist starts with `#EXTM3U` and declares `#EXT-X-VERSION:3` or higher.
- Rewriting covers every URI form: variant lines and `URI=` attributes of `EXT-X-MEDIA`,
  `EXT-X-KEY` and `EXT-X-MAP`.
- Subtitle tracks (`inject_subtitles`) are declared with `#EXT-X-MEDIA:TYPE=SUBTITLES` in group
  `subs` **before** the `#EXT-X-STREAM-INF` lines, and each variant then references
  `SUBTITLES="subs"`.

## Security

- No upstream URL appears in a locator, a response header or a log line (logs reduce URLs to
  `scheme://host`).
- The proxy is not an open relay: it refuses upstreams the session did not name, and local or
  DNS-resolved private upstreams.

## Pitfalls

- Buffering entire manifests: VOD manifests with many segments can be large.
- Following playlist update cycles in the proxy: it fetches on demand.
