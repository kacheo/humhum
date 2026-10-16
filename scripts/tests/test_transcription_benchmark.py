"""Regression benchmark scoring and integrity gates."""
import importlib.util
from pathlib import Path
import unittest

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
