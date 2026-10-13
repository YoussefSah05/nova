"""Build caption-sized evaluation windows from LibriSpeech test-clean.

Input: the `test_clean` parquet of the `gilkeyio/librispeech-alignments`
dataset (revision 0daa1eb43dda38ee6ce752e785555380e5628f5c, CC BY 4.0),
whose word boundaries come from the Montreal Forced Aligner.

Output: NAME.wav (16 kHz mono 16-bit) + NAME.json ({"words": [...]})
pairs of about 30 s each: random utterances joined by 0.9 s pauses, as
`crates/align/examples/timing_eval.rs` expects.

    python3 prepare.py test_clean.parquet OUT_DIR [--utterances 40] [--seed 7]

Requires pyarrow and ffmpeg on PATH.
"""

import argparse
import json
import random
import subprocess
import tempfile
import wave
from pathlib import Path

import pyarrow.parquet as pq

RATE = 16_000
GAP = int(0.9 * RATE)
BUDGET = 30.0


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("parquet")
    parser.add_argument("out")
    parser.add_argument("--utterances", type=int, default=40)
    parser.add_argument("--seed", type=int, default=7)
    args = parser.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)

    source = pq.ParquetFile(args.parquet)
    rows = []
    for group in range(source.num_row_groups):
        rows.extend(source.read_row_group(group).to_pylist())
        if len(rows) > 400:
            break
    random.seed(args.seed)
    picked = random.sample(rows, args.utterances)

    utterances = []
    with tempfile.TemporaryDirectory() as tmp:
        for i, row in enumerate(sorted(picked, key=lambda r: r["id"])):
            audio = row["audio"]
            flac = Path(tmp) / f"{i}.flac"
            flac.write_bytes(audio["bytes"] if isinstance(audio, dict) else audio)
            pcm = subprocess.run(
                ["ffmpeg", "-nostdin", "-loglevel", "error", "-i", str(flac),
                 "-ar", str(RATE), "-ac", "1", "-f", "s16le", "-"],
                check=True, capture_output=True,
            ).stdout
            words = [w for w in row["words"] if w["word"]]
            if len(pcm) / 2 / RATE <= 28:
                utterances.append((pcm, words))

    windows, current, length = [], [], 0.0
    for pcm, words in utterances:
        duration = len(pcm) / 2 / RATE
        if current and length + duration > BUDGET:
            windows.append(current)
            current, length = [], 0.0
        current.append((pcm, words))
        length += duration + GAP / RATE
    if current:
        windows.append(current)

    for n, window in enumerate(windows):
        audio, words, t = b"", [], 0.0
        for pcm, utterance_words in window:
            words += [{"word": w["word"], "start": w["start"] + t, "end": w["end"] + t}
                      for w in utterance_words]
            audio += pcm + b"\0\0" * GAP
            t += len(pcm) / 2 / RATE + GAP / RATE
        with wave.open(str(out / f"w{n:02d}.wav"), "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(RATE)
            w.writeframes(audio)
        (out / f"w{n:02d}.json").write_text(json.dumps({"words": words, "duration": t}))
    print(f"{len(windows)} windows, {sum(len(u[1]) for u in utterances)} reference words")


if __name__ == "__main__":
    main()
