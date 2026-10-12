# Translation pipeline

How Nova turns source-language captions into English, French or Arabic captions. The user-facing
summary is in [On-device intelligence](features/intelligence.md); the long-term design is §38 of
the [master bootstrap](architecture/long-term-architecture.md).

## Why two stages

Whisper's built-in `translate` task only produces English. Nova therefore transcribes in the
spoken language first, then translates the committed text with a dedicated model. This works for
every supported pair and lets source and translated captions share timing.

## Engine

- **Models:** four [Opus-MT](https://github.com/Helsinki-NLP/Opus-MT) Marian models (en→fr,
  fr→en, en→ar, ar→en), int8-quantized ONNX exports from the
  [Xenova](https://huggingface.co/Xenova) conversions, CC-BY-4.0.
- **Runtime:** the app's single ONNX Runtime (`crates/translate`, greedy decoding). An earlier
  NLLB/CTranslate2 engine was removed because its protobuf could not coexist with ONNX Runtime.
- **Manifest:** `apps/desktop/src-tauri/resources/models.manifest.json` pins each model to a
  Hugging Face commit and lists every file's size and SHA-256. Routing reads the manifest, never
  hard-coded IDs.

## Routing

`ModelManifest::translation_route(source, target)` picks a direct model when one exists,
preferring streaming-class and freely licensed models. Otherwise it pivots through English
(for example fr→en→ar). A pivot holds one model lease at a time.

## Flow

1. The caption session commits Whisper segments (final text with word timings).
2. The player requests translation of up to **eight committed segment IDs** at a time
   (`translate_caption_segments`). The request is bound to the active caption session, so
   stopping playback or switching media, track or target cancels it.
3. Each segment's cache key combines the pipeline and decoding versions, the language pair, the
   SHA-256 of the source text, and the route's model IDs and revisions. Hits return immediately.
4. Misses run under a **Realtime** runtime lease, then are validated (same count, non-empty,
   bounded length) before being cached and returned.
5. On failure, the source captions remain on screen with an explanation and a retry.

## Installing models

`download_translation_models` installs every unverified direction. Each file streams to
`<file>.part`, is hashed while writing, is cut off past its pinned size, and is renamed only on
an exact match. The directory is then attested (`.nova-verified`), so later status checks compare
metadata instead of re-hashing. Only models that need work take the runtime lock, so a direction
in use for captions is never disturbed.

## Language support

| | → English | → French | → Arabic |
|---|---|---|---|
| English | — | direct | direct |
| French | direct | — | via English |
| Arabic | direct | via English | — |

Arabic captions render right-to-left with the same appearance settings as source captions.

## Not yet supported

- Translation of live-channel captions (needs live audio capture).
- Context-aware translation across segments (each segment is translated independently).
