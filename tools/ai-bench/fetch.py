#!/usr/bin/env python3
"""Fetch the AI benchmark's pinned models and evaluation data (AI22).

Everything goes under ROOT (outside the repo, ideally the external model SSD);
nothing is ever written into the repository. Each file is downloaded to a
temporary name, verified against the pinned checksum in manifests/, and only
then moved into place. Verified files already present are skipped, so an
interrupted run simply resumes.

    python3 tools/ai-bench/fetch.py /Volumes/Lexar/nova-ai/bench            # everything
    python3 tools/ai-bench/fetch.py ROOT --data                              # datasets only
    python3 tools/ai-bench/fetch.py ROOT --models whisper-small-q5_1,opus-mt-en-fr

Standard library only.
"""

import argparse
import csv
import hashlib
import json
import os
import shutil
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def git_sha1(path: Path) -> str:
    """Git blob hash, which Hugging Face publishes for small (non-LFS) files."""
    data = path.read_bytes()
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def verified(path: Path, entry: dict) -> bool:
    if not path.is_file():
        return False
    if "sha256" in entry:
        return sha256(path) == entry["sha256"]
    if "git_sha1" in entry:
        return git_sha1(path) == entry["git_sha1"]
    raise SystemExit(f"no checksum pinned for {entry['url']}")


def fetch(entry: dict, dest: Path) -> Path:
    if verified(dest, entry):
        print(f"  ok        {dest}")
        return dest
    dest.parent.mkdir(parents=True, exist_ok=True)
    partial = dest.with_name(dest.name + ".partial")
    size = entry.get("bytes")
    label = f" ({size / 1e6:.0f} MB)" if size else ""
    print(f"  download  {dest.name}{label}")
    request = urllib.request.Request(entry["url"], headers={"User-Agent": "nova-ai-bench"})
    with urllib.request.urlopen(request) as response, partial.open("wb") as out:
        shutil.copyfileobj(response, out, 1 << 20)
    if not verified(partial, entry):
        partial.unlink()
        raise SystemExit(f"checksum mismatch for {entry['url']}: refusing it")
    partial.replace(dest)
    return dest


def fetch_models(root: Path, only: set) -> None:
    manifest = json.loads((HERE / "manifests/models.json").read_text())
    for model in manifest["models"]:
        if only and model["id"] not in only:
            continue
        flag = " [non-commercial]" if model["nc"] else ""
        print(f"{model['id']} ({model['licence']}){flag}")
        for entry in model["files"]:
            path = fetch(entry, root / "models" / entry["path"])
            if path.suffix == ".zip":
                target = path.with_suffix("")  # foo.mlmodelc
                if not target.exists():
                    # Extract beside the target, then rename: an interrupted
                    # extraction never leaves a half-written model in place.
                    print(f"  unzip     {path.name}")
                    staging = path.parent / (target.name + ".partial")
                    shutil.rmtree(staging, ignore_errors=True)
                    with zipfile.ZipFile(path) as z:
                        z.extractall(staging)
                    inner = staging / target.name
                    (inner if inner.exists() else staging).rename(target)
                    shutil.rmtree(staging, ignore_errors=True)


def fetch_data(root: Path) -> None:
    manifest = json.loads((HERE / "manifests/datasets.json").read_text())
    flores = manifest["flores200"]
    print("FLORES-200 devtest")
    archive = fetch(flores, root / "downloads/flores200_dataset.tar.gz")
    devtest = root / "mt/flores200/devtest"
    devtest.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive) as tar:
        for member in tar.getmembers():
            name = Path(member.name).name
            if member.isfile() and "/devtest/" in member.name and name.split(".")[0] in flores["devtest"]:
                (devtest / name).write_bytes(tar.extractfile(member).read())

    for lang, files in manifest["fleurs"]["languages"].items():
        print(f"FLEURS test {lang}")
        tsv = fetch(files["tsv"], root / f"downloads/fleurs-{lang}-test.tsv")
        audio = fetch(files["audio"], root / f"downloads/fleurs-{lang}-test.tar.gz")
        out = root / f"asr/fleurs/{lang}"
        (out / "audio").mkdir(parents=True, exist_ok=True)
        with tarfile.open(audio) as tar:
            for member in tar.getmembers():
                if member.isfile() and member.name.endswith(".wav"):
                    target = out / "audio" / Path(member.name).name
                    if not target.exists():
                        target.write_bytes(tar.extractfile(member).read())
        # FLEURS TSV columns: id, file name, raw transcription, normalized
        # transcription, ... One line per recording; keep the raw text (the
        # scorer normalizes both sides the same way).
        with tsv.open(newline="", encoding="utf-8") as f, (out / "manifest.jsonl").open("w", encoding="utf-8") as m:
            for row in csv.reader(f, delimiter="\t", quoting=csv.QUOTE_NONE):
                if len(row) < 3 or not (out / "audio" / row[1]).exists():
                    continue
                m.write(json.dumps({"id": row[0] + "-" + row[1], "audio": f"audio/{row[1]}", "text": row[2]}, ensure_ascii=False) + "\n")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("root", type=Path, help="destination, outside the repository")
    parser.add_argument("--data", action="store_true", help="datasets only")
    parser.add_argument("--models", default=None, help="comma-separated model ids (default: all)")
    args = parser.parse_args()
    root = args.root.resolve()
    repo = HERE.parent.parent.resolve()
    if root == repo or repo in root.parents:
        sys.exit("ROOT must be outside the repository")
    if args.data or args.models is None:
        fetch_data(root)
    if not args.data:
        fetch_models(root, set(filter(None, (args.models or "").split(","))))
    print(f"done: {root}")


if __name__ == "__main__":
    main()
