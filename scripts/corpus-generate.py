#!/usr/bin/env python3
"""Build the playback corpus (umbrella issue #4) from its manifest.

Reads `crates/oxideav-tests/corpus/manifest.csv` (id, ext, section,
title, ffmpeg_args) and writes one fixture per row as `<dir>/<id><ext>`
with the black-box `ffmpeg` binary. Fixtures are ~2 s synthetic
signals (`testsrc2` / `sine` / `aevalsrc`), so the whole set is a few
hundred MB and never goes into git.

    scripts/corpus-generate.py /path/to/corpus            # all 245
    scripts/corpus-generate.py /path/to/corpus --only 001,064,236
    scripts/corpus-generate.py /path/to/corpus --force -j 4

A row whose ffmpeg command fails (an encoder this ffmpeg build lacks,
e.g. libtheora / libaom) is reported and left absent; the harness in
`crates/oxideav-tests/tests/corpus.rs` records it as MISSING. The
failures are listed in `<dir>/generate.log`.
"""
import argparse
import csv
import os
import shlex
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_MANIFEST = os.path.join(
    HERE, "..", "crates", "oxideav-tests", "corpus", "manifest.csv"
)


def load_manifest(path):
    with open(path, newline="", encoding="utf-8") as f:
        rows = list(csv.DictReader(f))
    for key in ("id", "ext", "section", "title", "ffmpeg_args"):
        if rows and key not in rows[0]:
            sys.exit(f"manifest {path}: missing column {key!r}")
    return rows


def build_one(row, out_dir, ffmpeg, force):
    out = os.path.join(out_dir, row["id"] + row["ext"])
    if os.path.exists(out) and os.path.getsize(out) > 0 and not force:
        return row["id"], "kept", ""
    # ffmpeg picks the muxer from the extension, so the staging name
    # keeps it (`001.part.mp4`); rows that need an explicit muxer
    # (raw elementary streams, MPEG-PS) carry `-f` in the manifest.
    tmp = os.path.join(out_dir, row["id"] + ".part" + row["ext"])
    cmd = [ffmpeg, "-hide_banner", "-nostdin", "-v", "error", "-y"]
    cmd += shlex.split(row["ffmpeg_args"])
    cmd.append(tmp)
    try:
        proc = subprocess.run(
            cmd, capture_output=True, text=True, timeout=600, check=False
        )
    except subprocess.TimeoutExpired:
        _unlink(tmp)
        return row["id"], "failed", "ffmpeg timed out after 600 s"
    if proc.returncode != 0 or not os.path.exists(tmp) or os.path.getsize(tmp) == 0:
        _unlink(tmp)
        tail = (proc.stderr or "").strip().splitlines()
        return row["id"], "failed", " | ".join(tail[-3:]) if tail else f"exit {proc.returncode}"
    os.replace(tmp, out)
    return row["id"], "built", ""


def _unlink(path):
    try:
        os.unlink(path)
    except FileNotFoundError:
        pass


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("out_dir", help="directory that receives <id><ext> fixtures")
    ap.add_argument("--manifest", default=DEFAULT_MANIFEST)
    ap.add_argument("--ffmpeg", default=os.environ.get("FFMPEG", "ffmpeg"))
    ap.add_argument("--only", default="", help="comma-separated fixture ids")
    ap.add_argument("--force", action="store_true", help="rebuild existing fixtures")
    ap.add_argument("-j", "--jobs", type=int, default=2, help="parallel ffmpeg runs")
    args = ap.parse_args()

    rows = load_manifest(args.manifest)
    if args.only:
        wanted = {x.strip().zfill(3) for x in args.only.split(",") if x.strip()}
        rows = [r for r in rows if r["id"] in wanted]
    os.makedirs(args.out_dir, exist_ok=True)

    results = []
    with ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        futures = [
            pool.submit(build_one, r, args.out_dir, args.ffmpeg, args.force) for r in rows
        ]
        for fut in futures:
            fid, state, why = fut.result()
            results.append((fid, state, why))
            if state == "failed":
                print(f"{fid}: FAILED {why}", flush=True)

    built = sum(1 for _, s, _ in results if s == "built")
    kept = sum(1 for _, s, _ in results if s == "kept")
    failed = [(f, w) for f, s, w in results if s == "failed"]
    log = os.path.join(args.out_dir, "generate.log")
    with open(log, "w", encoding="utf-8") as f:
        for fid, why in failed:
            f.write(f"{fid}\t{why}\n")
    print(f"corpus: {built} built, {kept} kept, {len(failed)} failed -> {args.out_dir}")
    return 1 if failed and not (built or kept) else 0


if __name__ == "__main__":
    sys.exit(main())
