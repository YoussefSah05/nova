# Translation Pipeline

> This document expands on ARCHITECTURE.md §9.
> **Status**: Phase 0 stub. Detailed runbook written in Phase 7.

## Overview

Nova's translation pipeline is a two-stage process: Whisper transcribes audio to text
in the source language, then a separate translation model converts that text to the
target language. This enables all language pairs, not just English.

See **ARCHITECTURE.md §9** for:
- Why we do not use Whisper's built-in `--task translate` (English-only limitation)
- Backend comparison table (NLLB-200 / DeepL / Google)
- VOD translation flow
- Live translation flow
- Caching strategy
- Dual mode (source + translation displayed together)
- Language defaults and sticky rules

## Language code mapping

NLLB-200 uses its own language codes (e.g., `arb_Arab` for Modern Standard Arabic, `fra_Latn`
for French, `eng_Latn` for English) rather than BCP-47 codes.
The mapping table lives in the translation notes.

## Model storage

Models are stored at:
```
~/Library/Application Support/Nova/models/nllb/
```

Downloaded and converted by `scripts/convert-nllb-models.sh` during first-run or via
**Settings → Models**.
