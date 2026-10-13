# Roadmap

Nova is **0.2.0-alpha**. This page summarizes direction; the authoritative milestone list and
what is verified are in the [implementation status](nova-implementation-status.md), against the
[long-term architecture](architecture/long-term-architecture.md).

## Now (shipped in 0.2.0-alpha)

- M3U, Xtream and XMLTV import at 40k+ entries; scheduled refresh
- Films and shows as canonical titles with TMDB enrichment, versions and audio languages
- Live channel families and variants, programme guide, variant health and memory
- Native playback (libmpv) with resume, autoplay, recovery, AirPlay and picture-in-picture
- On-device captions (Whisper) and translated captions (Opus-MT, EN · FR · AR)
- Word-accurate caption timing (Silero VAD + MMS forced alignment) and captions prepared
  before Play, with no cold-start wait
- Hybrid exact + semantic search; dialogue search within a title
- Model manager with verified downloads and external model folders
- 120 Hz interface on ProMotion displays

## Next

- **Live captions:** capture audio from the live player into the resident speech engine,
  with the same timing stage as films
- **Ask Nova (⌘K assistant):** typed, validated intents ("French films from the 90s",
  "resume my show")
- **About this title:** short summaries from a local LLM
- **Clear AI states:** per-feature messages for missing, disabled, offline or failing models
- **Smoother at 120 Hz:** lighter cards during fast scrolling, guided by overlay measurements

## Later

- Signed, notarised, self-contained builds (bundled media libraries)
- Catalog pipeline v2: lossless snapshots and incremental refresh (already built, not wired)
- Cross-source series and episode identity; manual match correction
- Accessibility pass with VoiceOver and full keyboard navigation
- Intel Mac support; Windows exploration

Ideas and requests are welcome as [GitHub issues](https://github.com/YoussefSah05/nova/issues).
