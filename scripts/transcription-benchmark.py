#!/usr/bin/env python3
"""Run the ignored production-path Rust benchmark and enforce regression gates."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess
import tempfile

MODEL_HASHES = {
    "base_en": ("ggml-base.en.bin", "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002"),
    "small_en": ("ggml-small.en.bin", "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d"),
}


def wer(reference, actual):
    def words(s):
        return re.sub(r"[^\w\s]", "", s.lower()).split()
    ref, hyp = words(reference), words(actual)
    row = list(range(len(hyp) + 1))
    for i, a in enumerate(ref, 1):
        nxt = [i]
        for j, b in enumerate(hyp, 1):
            nxt.append(min(nxt[-1] + 1, row[j] + 1, row[j-1] + (a != b)))
        row = nxt
    return row[-1], len(ref)


def regressions(base, candidate):
    limits = {"wer": base["wer"] + .02, "warm_ms": base["warm_ms"] * 1.10, "peak_rss_bytes": base["peak_rss_bytes"] * 1.20}
    return [key for key, limit in limits.items() if candidate[key] > limit + 1e-9]


def check_comparable(base, candidate):
    for key in ("corpus_sha256", "models", "profile", "machine", "backend", "rustc"):
        if base[key] != candidate[key]:
            raise ValueError("incomparable benchmark: " + key)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(args):
    corpus = args.corpus.resolve()
    manifest = json.loads((corpus / "manifest.json").read_text())
    hashes = {}
    for model, (filename, expected) in MODEL_HASHES.items():
        path = args.models / filename
        if digest(path) != expected:
            raise ValueError("model checksum mismatch: " + model)
        hashes[model] = expected
    for clip in manifest["clips"]:
        if digest(corpus / clip["file"]) != clip["sha256"]:
            raise ValueError("corpus checksum mismatch: " + clip["file"])
    result = {"corpus_sha256": digest(corpus / "manifest.json"), "models": hashes, "profile": "release", "machine": platform.platform() + ":" + subprocess.check_output(["sysctl", "-n", "machdep.cpu.brand_string"], text=True).strip(), "backend": "cpu", "engine": args.engine, "binary_sha256": digest(args.binary), "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(), "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(), "results": {}}
    for model, (filename, _) in MODEL_HASHES.items():
        clips, edits, words, warm, peak = [], 0, 0, [], 0
        for clip in manifest["clips"]:
            with tempfile.TemporaryDirectory(prefix="humhum-bench-") as temp:
                output = Path(temp) / "result.json"
                env = os.environ | {"HUMHUM_BENCH_MODEL": str((args.models / filename).resolve()), "HUMHUM_BENCH_PCM": str(corpus / clip["file"]), "HUMHUM_BENCH_OUTPUT": str(output)}
                subprocess.run([str(args.binary.resolve()), "transcription::benchmarks::real_model", "--exact", "--ignored", "--nocapture"], env=env, check=True, stdout=subprocess.DEVNULL)
                measured = json.loads(output.read_text())
            for sample in measured["runs"]:
                e, w = wer(clip["text"], sample["text"])
                if clip["kind"] == "speech":
                    edits += e
                    words += w
            if clip["kind"] == "speech":
                warm.extend(s["ms"] for s in measured["runs"][1:])
            peak = max(peak, measured["peak_rss_bytes"])
            clips.append(dict(clip=clip["id"], kind=clip["kind"], **measured))
            print(model, clip["id"], "complete", flush=True)
        result["results"][model] = {"wer": edits / words, "warm_ms": statistics.median(warm), "peak_rss_bytes": peak, "clips": clips}
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    if args.baseline:
        base = json.loads(args.baseline.read_text())
        check_comparable(base, result)
        failures = {}
        for model, measured in result["results"].items():
            bad = regressions(base["results"][model], measured)
            old = {c["clip"]: c for c in base["results"][model]["clips"]}
            for clip in measured["clips"]:
                for index, sample in enumerate(clip["runs"]):
                    previous = old[clip["clip"]]["runs"][index]["text"]
                    if clip["kind"] == "speech" and previous.strip() and not sample["text"].strip():
                        bad.append("new blank speech: " + clip["clip"])
                    if clip["kind"] != "speech" and not previous.strip() and sample["text"].strip():
                        bad.append("new nonspeech output: " + clip["clip"])
            if bad:
                failures[model] = bad
        print(json.dumps({"regressions": failures}, indent=2))
        if failures:
            raise SystemExit(1)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, default=Path("tests/transcription-corpus"))
    parser.add_argument("--models", type=Path, required=True)
    parser.add_argument("--engine", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--baseline", type=Path)
    run(parser.parse_args())
