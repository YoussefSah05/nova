# Caption timing and cold start — evaluation, 2026-10-13

Measurements behind [ADR 0014](../adr/0014-caption-timing-and-cold-start.md). Everything here was
run on an Apple Silicon MacBook Pro, with the models installed by Nova's own installer.

## 1. Word timing accuracy

**Question:** how far from the true word boundaries do caption word times land?

**Data:** LibriSpeech test-clean (CC BY 4.0) with word boundaries from the Montreal Forced Aligner,
as published in `gilkeyio/librispeech-alignments` (revision `0daa1eb4`). 40 random utterances
(seed 7) are joined with 0.9 s pauses into 13 windows of about 30 s, the size of a caption window:
**779 reference words**.

**Method:** each window is transcribed by the installed `whisper-cli` (large-v3-turbo q5_0,
`--max-len 60 --split-on-word`, language `en`) three ways. Hypothesis words are matched to
reference words by longest common subsequence on normalized text; errors are measured on matched
words only (756 of 779; the rest are recognition differences such as numbers written as digits).

```text
python3 tools/caption-timing-eval/prepare.py test_clean.parquet windows/
cargo run --release -p nova-align --example timing_eval -- \
    "$HOME/Library/Application Support/app.nova.desktop" \
    "$HOME/Library/Application Support/app.nova.desktop/models/align/mms-300m-forced-aligner" \
    "$HOME/Library/Application Support/app.nova.desktop/models/vad/silero-vad-v5" \
    windows/
```

| Word **start** error | Mean | Median | p90 | ≤ 100 ms | ≤ 200 ms |
|---|---|---|---|---|---|
| Whisper interpolated timestamps (what Nova shipped) | 398 ms | 240 ms | 990 ms | 26.2% | 45.0% |
| Whisper DTW (flash attention off) | 215 ms | 185 ms | 380 ms | 10.3% | 54.5% |
| **DTW + Silero VAD + MMS CTC alignment** | **50 ms** | **41 ms** | **91 ms** | **92.5%** | **99.3%** |

| Word **end** error | Mean | Median | p90 | ≤ 100 ms | ≤ 200 ms |
|---|---|---|---|---|---|
| Whisper interpolated timestamps | 401 ms | 260 ms | 980 ms | 24.7% | 43.8% |
| Whisper DTW | 174 ms | 60 ms | 500 ms | 64.0% | 79.2% |
| **DTW + VAD + CTC alignment** | **42 ms** | **25 ms** | **91 ms** | **91.5%** | **98.5%** |

Notes:

- DTW alone has a lower mean but fewer starts within 100 ms than the interpolated times: DTW gives
  each token one instant, which lands consistently ~150–200 ms late on word onsets. The CTC
  aligner removes that bias.
- Cost: the timing stage took 23.1 s of CPU for the 13 windows (~1.8 s per 30 s window, uint8
  model, four threads); the VAD 1.0 s in total.
- The q4 build of the aligner gave the same accuracy (50 ms mean) but took 36.5 s: uint8 ships.
- An earlier grouping of the same utterances gave 345 → 49 ms; the table above is the one the
  committed script reproduces.

### Design checks found by this evaluation

- **Per-segment alignment lost words.** Aligning each Whisper segment inside its own bounds
  (±0.6 s) dropped words whose segment Whisper had placed in the following pause ("servant." sat
  1.2 s late). Aligning the whole window jointly kept all of them.
- **"The aligner placed it" is not evidence of speech.** A hallucinated "Thank you." past the end
  of the audio was kept until the rule required a confidently aligned word on VAD speech, and
  every time was clamped to the analysed audio.

### French and Arabic

No reference boundaries exist yet for these. On FLEURS (`fr_fr`, `ar_eg`), every word aligned
(19/19 and 15/15) with plausible boundaries. Arabic example: Whisper DTW ran "الاستوائية" to
7.62 s, overlapping the next word, and "جيري" to 8.15 s, past the end of speech (VAD: 7.42 s); the
aligner placed them at 6.32–6.94 s and 7.06–7.32 s.

## 2. Time to first caption

**Question:** how long after a caption session starts does the first caption exist?

**Data:** 2 min of the same real speech (`caption_cold_start` example), run as a caption session
would: the old path starts one `whisper-cli` per 30 s window; the new path starts the resident
worker, decodes an 11 s first window, then 30 s windows, with the timing stage.

**Cold case:** a fresh copy of the Core ML encoder at a new path, so macOS must compile it, as on a
first run or after it evicts its compiled-model cache.

| | First caption | Later windows (27 s of new audio each) |
|---|---|---|
| Old: `whisper-cli` per window, cold Core ML | **32.1 s** | 1.6–2.1 s |
| New: resident worker, cold Core ML | **2.1 s** (worker ready on Metal in 0.24 s) | 3.6–4.2 s incl. ~2.3 s alignment |
| Old, everything cached | 2.6 s | 1.6–2.1 s |
| New, everything cached | 2.4 s | 3.7–4.5 s |

Network time to fetch audio is not included (it is the same for both paths, and smaller for the
short first window). Later windows run ahead of playback, inside the 90 s prefetch.

## 3. In the app, on real films

Release build, a separate *Demo* profile whose only playlist is three Blender open movies
(*Elephants Dream*, *Sintel*, *Tears of Steel*, CC BY) streamed from download.blender.org.

- **Installer:** Settings → Models → Caption timing downloaded both models from their pinned
  revisions, verified their SHA-256 and showed "2 of 2 installed".
- **Prepare before Play:** opening *Elephants Dream*'s playback menu started the speech worker
  (Metal) and a prepared session. Before Play was pressed, three windows had run:
  - window 1, the opening music: the VAD heard no speech, Whisper was not run;
  - windows 2 and 3: 28 and 17 words force-aligned; 1 and 2 invented segments dropped.
- **Adoption:** "Play with transcription" adopted the session (the CC menu read "Captions
  prepared before playback") and transcription continued past the prepared 45 s instead of
  starting over. The stored transcript began "At the left we can see..." at 15.17 s, as spoken.

### Bugs found and fixed during this run

- After adoption, the session loop kept emitting under the title page's session id, so the
  player ignored later cues and status (the CC menu stayed on "prepared before playback"). Events
  now read the session's shared identity.
- Aligned cues ended exactly on their last word, so short lines ("Emo?", 0.18 s) were on screen
  too briefly to read, and the overlay only shows a cue inside `[start, end)`. Cues now stay up at
  least 1 s and 0.4 s after their last word, never overlapping the next one; word times are
  unchanged.

- With reading time added, a cue near a window boundary could still be on screen when the next
  window's first cue started, and the overlay showed the first active cue, hiding the new line's
  start. It now shows the cue that started last.

After the first fix, a second run on *Tears of Steel* (release build) showed captions on every
sampled dialogue frame, prepared before Play (first window done 3 s after the menu opened,
adopted with its cues), then French "Source + translation" ("I heard you guys talking last
night." / "Je vous ai entendu parler hier soir."), dialogue search with a jump to the matching
line, and Arabic right-to-left captions. That run is the
[recorded demo](../assets/demo/nova-intelligence-demo.mp4). It also shows a translation limit:
for "Are you ready? Of course you're ready. You're a rockstar." Opus-MT returned only
"Vous êtes prêt ?".

That run predates two later corrections, so the recording shows cues without reading-time
padding: an independent review then found that the padding was computed in the worker but lost
on the way to the session (it is now applied to the stored cues, after stitching), and the
overlapping-cue fix above landed after the recording too.

Not caused by this work, noted for follow-up: in Settings, "Remove source" (and four other
actions) use `window.confirm`, which returns false in the macOS WebView without showing a dialog,
so those actions do nothing. In the profile picker's manage mode, the delete button removes a
profile at once, without confirmation.
