#!/usr/bin/env python3
"""Unit tests for layout_envelope.py. Stdlib only: `python3 -m unittest` from
benchmarks/history/, or `python3 test_layout_envelope.py`."""

from __future__ import annotations

import io
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import layout_envelope as le
from parse_criterion import parse_benchmarks


def criterion_text(benches: dict[str, tuple[float, float]]) -> str:
    """Render a minimal criterion stdout block per benchmark.

    `benches` maps a long-enough id (printed on its own line by criterion) to
    (median_ns, throughput_mib_s). The lo/hi values bracket the mid; only the
    mid is parsed back.
    """
    out = []
    for bench, (ns, thrpt) in benches.items():
        us = ns / 1000.0
        out.append(bench)
        out.append(f"                        time:   [{us*0.99:.4f} us {us:.4f} us {us*1.01:.4f} us]")
        out.append(
            f"                        thrpt:  [{thrpt*0.99:.2f} MiB/s {thrpt:.2f} MiB/s {thrpt*1.01:.2f} MiB/s]"
        )
    return "\n".join(out) + "\n"


class PercentileTests(unittest.TestCase):
    def test_edges_and_interpolation(self):
        self.assertEqual(le.percentile([], 50), 0.0)
        self.assertEqual(le.percentile([7.0], 90), 7.0)
        self.assertAlmostEqual(le.percentile([0.0, 10.0], 50), 5.0)
        self.assertAlmostEqual(le.percentile([0.0, 10.0], 90), 9.0)
        self.assertAlmostEqual(le.percentile([1.0, 2.0, 3.0], 100), 3.0)


class EnvelopeTests(unittest.TestCase):
    def setUp(self):
        # Bench A swings hard across layouts; bench B barely moves.
        self.runs = {
            "cgu1": {
                "buffa/group/alpha": {"throughput_mib_s": 100.0, "median_ns": 100.0},
                "buffa/group/beta": {"throughput_mib_s": 200.0, "median_ns": 50.0},
            },
            "cgu16": {
                "buffa/group/alpha": {"throughput_mib_s": 120.0, "median_ns": 83.0},
                "buffa/group/beta": {"throughput_mib_s": 204.0, "median_ns": 49.0},
            },
        }

    def test_range_pct(self):
        env = le.compute_envelope(self.runs, "throughput_mib_s")
        self.assertAlmostEqual(env["buffa/group/alpha"]["range_pct"], 18.18, places=2)
        self.assertAlmostEqual(env["buffa/group/beta"]["range_pct"], 1.98, places=2)
        self.assertEqual(env["buffa/group/alpha"]["min"], 100.0)
        self.assertEqual(env["buffa/group/alpha"]["max"], 120.0)

    def test_single_run_benchmarks_excluded(self):
        self.runs["cgu1"]["buffa/group/orphan"] = {"throughput_mib_s": 5.0}
        env = le.compute_envelope(self.runs, "throughput_mib_s")
        self.assertNotIn("buffa/group/orphan", env)

    def test_summary_distribution(self):
        env = le.compute_envelope(self.runs, "throughput_mib_s")
        summary = le.summarize(env)
        self.assertEqual(summary["benchmarks"], 2)
        self.assertAlmostEqual(summary["max_range_pct"], 18.18, places=2)
        self.assertAlmostEqual(summary["p50_range_pct"], 10.08, places=2)

    def test_metric_switch(self):
        env = le.compute_envelope(self.runs, "median_ns")
        # alpha: (100-83)/median(100,83)=91.5 -> 18.58%
        self.assertAlmostEqual(env["buffa/group/alpha"]["range_pct"], 18.58, places=2)


class IntegrationTests(unittest.TestCase):
    def test_parse_then_envelope_roundtrip(self):
        t1 = criterion_text({"buffa/longname/decode_view": (250.0, 800.0)})
        t2 = criterion_text({"buffa/longname/decode_view": (305.0, 650.0)})
        runs = {"cgu1": parse_benchmarks(t1), "cgu16": parse_benchmarks(t2)}
        env = le.compute_envelope(runs, "throughput_mib_s")
        # (800-650)/median(800,650)=725 -> 20.69%
        self.assertAlmostEqual(
            env["buffa/longname/decode_view"]["range_pct"], 20.69, places=2
        )

    def test_main_requires_two_runs(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "one.txt"
            p.write_text(criterion_text({"buffa/longname/decode": (10.0, 10.0)}))
            self.assertEqual(le.main([f"--run=a={p}"]), 2)

    def test_main_emits_markdown(self):
        with tempfile.TemporaryDirectory() as d:
            p1, p2 = Path(d) / "a.txt", Path(d) / "b.txt"
            p1.write_text(criterion_text({"buffa/longname/decode": (10.0, 100.0)}))
            p2.write_text(criterion_text({"buffa/longname/decode": (12.0, 120.0)}))
            buf = io.StringIO()
            with redirect_stdout(buf):
                rc = le.main([f"--run=cgu1={p1}", f"--run=cgu16={p2}"])
            self.assertEqual(rc, 0)
            md = buf.getvalue()
            self.assertIn("layout-noise envelope", md)
            self.assertIn("buffa/longname/decode", md)


if __name__ == "__main__":
    unittest.main()
