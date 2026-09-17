# Synthetic IPTV corpus v1

All content is hand-authored test metadata. `fixture / fixture` are dummy credentials,
not a provider account. No provider exports, private URLs, video or external artwork
are included. The service has no upstream proxy and serves only compiled-in routes.

Normal catalog: 2 numbered live channels, 5 movie occurrences and 1 series. Movie
201 appears twice with conflicting names to expose upstream-ID collision handling.
Dune (2021) quality/language variants are same-work candidates; the 1984 remake and
Part Two must remain distinct. Numbered sports channels must not be merged. These
are annotations for the future compiler corpus, not assertions that current import
performs correct canonical grouping. French/Arabic mixed text is deliberate.

The M3U includes a concrete S01E02 episode; Xtream's series list has an unfetched
series container and a separate episode endpoint, including season-zero specials
and mixed numeric/string IDs. Tests characterize the legacy sentinel (0,0) without
promoting it to the canonical numbering model.

Raw JSON/M3U contain unknown fields deliberately absent from legacy ParsedEntry.
Preserve these fixtures as raw snapshot inputs; do not reconstruct lossless evidence
from ParsedEntry or replace this corpus with normalized output.

XMLTV time is fixed to 17 September 2026. The two programmes use different offsets
for the same UTC interval. No test depends on current wall-clock Now/Next behavior.

Scenarios:

| Name | Behavior |
|---|---|
| normal | Complete static catalog/guide responses |
| vod-unavailable | VOD returns 503; live and series still finish |
| vod-malformed | VOD returns invalid JSON with status 200 |
| vod-empty | VOD returns a valid empty array |
| playlist-html | M3U endpoint returns an HTML login-shaped body |

Each server instance has one immutable scenario and an ephemeral loopback port.
Only the `{{BASE_URL}}` placeholder in M3U is expanded to that port. Raw templates
stay deterministic; served playlist bytes naturally differ by port. Snapshot
fingerprints should hash actual captured bytes, not silently erase the port.

Version the corpus directory when deliberately changing established semantics.
Additive scenarios can remain v1; never rebaseline identity expectations just to
make an incorrect compiler pass. Scale/stream faults and licensed playable media
are separate upcoming fixtures. Current media paths intentionally return 404.
