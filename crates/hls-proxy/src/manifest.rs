//! HLS manifest rewriter.
//!
//! Takes an upstream `.m3u8` body and rewrites every segment / sub-playlist
//! URI so the player fetches them through the proxy. This gives Phase 6/7
//! a place to inject `#EXT-X-MEDIA:TYPE=SUBTITLES` lines without the player
//! ever knowing the upstream URLs.
//!
//! Phase 5 only rewrites; the subtitle injection seam ([`inject_subtitles`])
//! is a no-op until callers have VTT URLs to feed it.

use std::sync::OnceLock;

/// Rewrite every URI in `body` to what `proxied` maps its absolute URL to
/// (the proxy's session-scoped resource URL).
///
/// `base_url` is the absolute URL of the manifest itself; relative URIs in
/// the playlist are resolved against it before being mapped.
pub fn rewrite_manifest(
    body: &str,
    base_url: &str,
    mut proxied: impl FnMut(&str) -> String,
) -> String {
    // A byte-order mark would hide the first line's `#EXTM3U` tag.
    let body = body.strip_prefix('\u{feff}').unwrap_or(body);
    let mut out = String::with_capacity(body.len() + 256);
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            out.push_str(line);
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('#') {
            // Tag line. Most tags pass through unchanged, but a few embed
            // URIs (EXT-X-MEDIA URI=, EXT-X-KEY URI=, EXT-X-MAP URI=,
            // EXT-X-I-FRAME-STREAM-INF URI=) - those need their URI=
            // attribute rewritten too.
            if let Some(rewritten) = rewrite_tag_uri(rest, base_url, &mut proxied) {
                out.push('#');
                out.push_str(&rewritten);
                if line.ends_with('\n') {
                    out.push('\n');
                }
            } else {
                out.push_str(line);
            }
            continue;
        }
        // URI line - rewrite.
        let absolute = resolve(base_url, trimmed);
        out.push_str(&proxied(&absolute));
        if line.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// Subtitle-track injection seam used in Phase 6/7 to add caption tracks to
/// the master playlist. In Phase 5 the caller has no tracks to inject so
/// this is a no-op; the function exists now so future code drops in without
/// touching the player.
pub fn inject_subtitles(manifest: String, tracks: &[SubtitleTrack]) -> String {
    if tracks.is_empty() {
        return manifest;
    }
    let mut out = String::with_capacity(manifest.len() + tracks.len() * 120);
    let mut header_done = false;
    for line in manifest.split_inclusive('\n') {
        out.push_str(line);
        if !header_done && line.starts_with("#EXT-X-VERSION") {
            header_done = true;
            for t in tracks {
                out.push_str(&format!(
                    "#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"{}\",LANGUAGE=\"{}\",DEFAULT={},AUTOSELECT=YES,FORCED=NO,URI=\"{}\"\n",
                    escape_attr(&t.name),
                    escape_attr(&t.language),
                    if t.default { "YES" } else { "NO" },
                    escape_attr(&t.uri),
                ));
            }
        }
    }
    out
}

/// One subtitle track for [`inject_subtitles`].
pub struct SubtitleTrack {
    pub name: String,
    pub language: String,
    pub uri: String,
    pub default: bool,
}

fn rewrite_tag_uri(
    tag: &str,
    base_url: &str,
    proxied: &mut impl FnMut(&str) -> String,
) -> Option<String> {
    // Look for `URI="..."` and rewrite the value if present.
    let re = uri_attr_regex();
    let caps = re.captures(tag)?;
    let m = caps.get(1)?;
    let absolute = resolve(base_url, m.as_str());
    let proxied = proxied(&absolute);
    let mut out = String::with_capacity(tag.len() + proxied.len());
    out.push_str(&tag[..m.start()]);
    out.push_str(&proxied);
    out.push_str(&tag[m.end()..]);
    Some(out)
}

/// Lazy-compiled regex for `URI="..."` attribute extraction. We keep the
/// dependency surface tiny by hand-rolling a minimal matcher rather than
/// pulling `regex`. The HLS spec restricts URI= to a quoted string with
/// any non-quote character inside.
fn uri_attr_regex() -> &'static UriRegex {
    static R: OnceLock<UriRegex> = OnceLock::new();
    R.get_or_init(UriRegex::new)
}

struct UriRegex;
impl UriRegex {
    fn new() -> Self {
        Self
    }
    fn captures<'a>(&self, input: &'a str) -> Option<UriCapture<'a>> {
        // Find `URI="..."`. Quotes inside URIs aren't allowed by the spec,
        // so a simple scan suffices.
        let key = "URI=\"";
        let start = input.find(key)? + key.len();
        let rest = &input[start..];
        let end_rel = rest.find('"')?;
        Some(UriCapture {
            input,
            start,
            end: start + end_rel,
        })
    }
}

struct UriCapture<'a> {
    input: &'a str,
    start: usize,
    end: usize,
}

impl<'a> UriCapture<'a> {
    fn get(&self, idx: usize) -> Option<UriMatch<'a>> {
        match idx {
            1 => Some(UriMatch {
                s: &self.input[self.start..self.end],
                start: self.start,
                end: self.end,
            }),
            _ => None,
        }
    }
}

struct UriMatch<'a> {
    s: &'a str,
    start: usize,
    end: usize,
}

impl<'a> UriMatch<'a> {
    fn as_str(&self) -> &str {
        self.s
    }
    fn start(&self) -> usize {
        self.start
    }
    fn end(&self) -> usize {
        self.end
    }
}

fn resolve(base: &str, target: &str) -> String {
    if target.starts_with("http://") || target.starts_with("https://") {
        return target.to_string();
    }
    // Best-effort base join: strip filename from base, append target.
    if let Ok(parsed) = reqwest::Url::parse(base) {
        if let Ok(joined) = parsed.join(target) {
            return joined.to_string();
        }
    }
    target.to_string()
}

fn escape_attr(s: &str) -> String {
    s.replace('"', "%22")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_segment_lines() {
        let manifest =
            "#EXTM3U\n#EXTINF:6,\nseg-001.ts\n#EXTINF:6,\nhttps://cdn.example.com/seg-002.ts\n";
        let base = "https://provider.com/path/playlist.m3u8";
        let out = rewrite_manifest(manifest, base, |u| format!("P[{u}]"));
        assert!(out.contains("P[https://provider.com/path/seg-001.ts]"));
        assert!(out.contains("P[https://cdn.example.com/seg-002.ts]"));
    }

    #[test]
    fn rewrites_uri_attributes() {
        let manifest = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x0\n";
        let base = "https://provider.com/playlist.m3u8";
        let out = rewrite_manifest(manifest, base, |u| format!("P[{u}]"));
        assert!(out.contains("URI=\"P[https://provider.com/key.bin]\""));
    }

    #[test]
    fn passes_unchanged_tags_through() {
        let manifest = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n";
        let out = rewrite_manifest(manifest, "https://x.com/a.m3u8", |u| format!("P[{u}]"));
        assert!(out.contains("#EXT-X-TARGETDURATION:6"));
    }

    #[test]
    fn subtitle_injection_is_noop_without_tracks() {
        let m = "#EXTM3U\n#EXT-X-VERSION:3\n".to_string();
        let out = inject_subtitles(m.clone(), &[]);
        assert_eq!(out, m);
    }

    #[test]
    fn subtitle_injection_adds_media_tag() {
        let m = "#EXTM3U\n#EXT-X-VERSION:3\n".to_string();
        let tracks = vec![SubtitleTrack {
            name: "English".into(),
            language: "en".into(),
            uri: "http://127.0.0.1:1/captions/a/en.vtt".into(),
            default: true,
        }];
        let out = inject_subtitles(m, &tracks);
        assert!(out.contains("#EXT-X-MEDIA:TYPE=SUBTITLES"));
        assert!(out.contains("LANGUAGE=\"en\""));
        assert!(out.contains("DEFAULT=YES"));
    }
}
