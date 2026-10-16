"""Regression benchmark scoring and integrity gates."""
import importlib.util
from pathlib import Path
import unittest
import contextlib
import io
import json
import tempfile
from types import SimpleNamespace
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("benchmark", Path(__file__).parents[1] / "transcription-benchmark.py")
benchmark = importlib.util.module_from_spec(spec)
if spec.loader:
    spec.loader.exec_module(benchmark)


class BenchmarkTests(unittest.TestCase):
    def test_wer_normalizes_case_punctuation_and_whitespace(self):
        self.assertEqual(benchmark.wer("Hello, world!", "hello   WORLD"), (0, 2))
        self.assertEqual(benchmark.wer("one two three", "one four"), (2, 3))

    def test_gate_rejects_each_regression_without_hiding_others(self):
        base = {"wer": .1, "warm_ms": 100., "peak_rss_bytes": 1000}
        self.assertEqual(benchmark.regressions(base, base), [])
        self.assertEqual(set(benchmark.regressions(base, {"wer": .13, "warm_ms": 111., "peak_rss_bytes": 1201})), {"wer", "warm_ms", "peak_rss_bytes"})

    def test_comparison_refuses_different_inputs_or_build_profile(self):
        base = {"corpus_sha256": "a", "models": {"base_en": "b"}, "profile": "release", "machine": "M1", "backend": "cpu", "rustc": "rustc 1.88.0", "build": {"flags": "", "target": "aarch64-apple-darwin"}}
        benchmark.check_comparable(base, base)
        for key in base:
            changed = dict(base, **{key: "different"})
            with self.assertRaises(ValueError, msg=key):
                benchmark.check_comparable(base, changed)


class SmokeTests(unittest.TestCase):
    def test_smoke_runs_selected_clips_once_and_reports_no_warm_statistics(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "model.bin").write_bytes(b"model")
            (root / "speech.pcm").write_bytes(b"\0\0")
            (root / "noise.pcm").write_bytes(b"\0\0")
            clips = [dict(id=name, file=name + ".pcm", kind=kind, text=text,
                          sha256=benchmark.digest(root / (name + ".pcm")))
                     for name, kind, text in [("speech", "speech", "Hello world."), ("noise", "nonspeech", "")]]
            (root / "manifest.json").write_text(json.dumps(dict(clips=clips)))
            binary = root / "fake-inference"
            binary.write_text("""#!/usr/bin/env python3
import json, os
from pathlib import Path
assert os.environ['HUMHUM_BENCH_SMOKE'] == '1'
Path(os.environ['HUMHUM_BENCH_OUTPUT']).write_text(json.dumps({
    'rustc': 'test', 'build': {}, 'backend': {}, 'load_ms': 1,
    'peak_rss_bytes': 123, 'runs': [{'text': 'Hello world.', 'ms': 5}]}))
""")
            binary.chmod(0o755)
            args = SimpleNamespace(corpus=root, models=root, binary=binary, engine="test",
                                   output=root / "out.json", baseline=None, smoke=True,
                                   clips=["speech"], model="base_en")
            hashes = {"base_en": ("model.bin", benchmark.digest(root / "model.bin"))}
            with patch.object(benchmark, "MODEL_HASHES", hashes), contextlib.redirect_stdout(io.StringIO()):
                benchmark.run(args)
            report = json.loads(args.output.read_text())
            self.assertEqual(report["mode"], "smoke")
            result = report["results"]["base_en"]
            self.assertEqual([c["clip"] for c in result["clips"]], ["speech"])
            self.assertEqual(len(result["clips"][0]["runs"]), 1)
            self.assertEqual(result["single_pass_ms"], 5)
            self.assertNotIn("warm_ms", result)

    def test_smoke_cannot_be_used_as_a_paired_benchmark(self):
        args = SimpleNamespace(smoke=True, baseline=Path("baseline.json"), clips=None, model=None)
        with self.assertRaisesRegex(ValueError, "benchmark"):
            benchmark.run(args)

    def test_smoke_report_cannot_be_reused_as_benchmark_baseline(self):
        with self.assertRaisesRegex(ValueError, "smoke"):
            benchmark.check_comparable({"mode": "smoke"}, {})
