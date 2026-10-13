# Frequently asked questions

### Does Nova provide channels, films or a subscription?

No. Nova is a player. You bring your own M3U or Xtream playlist from a provider you are entitled
to use; Nova organises and plays it.

### Which playlists work?

M3U and M3U8 files or URLs, Xtream Codes accounts (server, username, password), and XMLTV
programme guides. Large playlists are fine: Nova is tested on one with about 42,000 entries.

### Does anything leave my Mac?

Only what you ask for: streams and playlists from your provider, TMDB data if you enable it with
your own key, and model downloads when you click them. There is no telemetry and no account.
Captions, translations and search run locally. See [SECURITY.md](../SECURITY.md).

### Which Macs are supported?

Apple Silicon Macs on macOS 14 Sonoma or newer. Intel Macs, Windows and Linux are not supported
yet.

### Why does Nova use a web interface instead of SwiftUI?

The interface runs in the system's WebKit through Tauri, while video (mpv) and all heavy work
(Rust) are native. That keeps Nova small, lets the design evolve quickly, and leaves the door
open to other platforms. The reasoning and trade-offs are in
[ADR 0013](adr/0013-web-ui-on-tauri-with-native-video.md).

### Does Nova run at 120 Hz?

Yes, on ProMotion displays, with **Settings → Appearance → High refresh rate** (on by default).
WebKit apps are capped at 60 fps unless they opt out; Nova does. Video always plays at its own
frame rate.

### How quickly do captions start?

Usually within a few seconds, often before the film itself. The speech model stays loaded in a
background process and decodes on the GPU while macOS prepares its Neural Engine version, so
even the very first run does not wait for that (measured: 2.1 s instead of 32 s). Opening a
title's playback menu, or opening a title when your profile starts captions on, transcribes the
opening before you press Play. Later 30-second windows are recognised ahead of playback.

### How accurate is caption timing?

With the optional **Caption timing** models (Settings → Models), every word is aligned to the
audio by a character-level model: on a reference benchmark, word starts landed 50 ms from the
true timing on average, against 398 ms for Whisper's own timestamps. A speech detector also
keeps captions off music and silence. Without these models, captions use Whisper's timing.

### Which caption languages are supported?

Speech recognition is configured for English, French and Arabic. Translation covers English ↔
French and English ↔ Arabic directly, and French ↔ Arabic through English.

### Can captions work on live TV?

Not yet. Live captions need audio capture from the live player; the speech engine for it exists
and wiring it is on the [roadmap](roadmap.md).

### How much disk space do the models need?

About 500 MB for captions, 450 MB for translation and 120 MB for catalog search. All are
optional, and they can live on an external drive.

### Is Nova free?

Nova is MIT-licensed. Model weights keep their own licences (Whisper: MIT; Opus-MT: CC-BY-4.0;
multilingual-E5: MIT).
