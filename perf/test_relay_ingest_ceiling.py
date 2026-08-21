#!/usr/bin/env python3
"""Unit tests for the ingest-ceiling harness verdict logic.

Every test that asserts a *pass* is paired with a mutant that must fail. A
harness whose contract cannot go red is decoration.
"""

from __future__ import annotations

import unittest

import relay_ingest_ceiling as harness


class SpreadTests(unittest.TestCase):
    def test_spread_is_relative_to_the_mean(self) -> None:
        self.assertAlmostEqual(harness.relative_spread([100.0, 102.0]), 2.0 / 101.0)

    def test_identical_runs_have_no_spread(self) -> None:
        self.assertEqual(harness.relative_spread([50.0, 50.0]), 0.0)

    def test_one_run_cannot_measure_a_spread(self) -> None:
        with self.assertRaises(ValueError):
            harness.relative_spread([50.0])

    def test_zero_throughput_cannot_measure_a_spread(self) -> None:
        # A run that accepted nothing has no scale to be noisy relative to;
        # silently returning 0.0 would hand back the tightest possible threshold
        # from the least trustworthy run.
        with self.assertRaises(ValueError):
            harness.relative_spread([0.0, 0.0])

    def test_threshold_widens_with_measured_noise(self) -> None:
        self.assertEqual(harness.knee_threshold(0.0), 1.0)
        self.assertAlmostEqual(harness.knee_threshold(0.01), 0.97)


class KneeTests(unittest.TestCase):
    def test_knee_is_the_first_persistent_shortfall(self) -> None:
        points = [(20.0, 1.0), (50.0, 1.0), (100.0, 0.80), (200.0, 0.41)]
        self.assertEqual(harness.find_knee(points, 0.99), 100.0)

    def test_a_lone_dip_is_not_a_knee(self) -> None:
        # MUTANT: one rate dips and the next recovers. Saturation is monotone, so
        # this is noise. A harness that called it a knee would report a ceiling
        # that moves run to run.
        points = [(20.0, 1.0), (50.0, 0.90), (100.0, 1.0), (200.0, 1.0)]
        self.assertIsNone(harness.find_knee(points, 0.99))

    def test_the_highest_rate_may_stand_alone(self) -> None:
        points = [(20.0, 1.0), (50.0, 1.0), (100.0, 0.70)]
        self.assertEqual(harness.find_knee(points, 0.99), 100.0)

    def test_no_shortfall_is_no_knee(self) -> None:
        points = [(20.0, 1.0), (50.0, 1.0), (100.0, 1.0)]
        self.assertIsNone(harness.find_knee(points, 0.99))

    def test_points_need_not_arrive_sorted(self) -> None:
        points = [(200.0, 0.41), (20.0, 1.0), (100.0, 0.80), (50.0, 1.0)]
        self.assertEqual(harness.find_knee(points, 0.99), 100.0)

    def test_bracket_spans_the_last_pass_and_the_knee(self) -> None:
        points = [(20.0, 1.0), (50.0, 1.0), (100.0, 0.80), (200.0, 0.41)]
        self.assertEqual(harness.knee_bracket(points, 0.99), (50.0, 100.0))

    def test_bracket_has_no_lower_bound_when_the_first_rate_saturates(self) -> None:
        points = [(20.0, 0.5), (50.0, 0.2)]
        self.assertEqual(harness.knee_bracket(points, 0.99), (None, 20.0))


class VerdictTests(unittest.TestCase):
    SATURATING = [(20.0, 1.0), (50.0, 1.0), (100.0, 0.80), (200.0, 0.41)]
    CLEAN = [(20.0, 1.0), (50.0, 1.0), (100.0, 1.0), (200.0, 1.0)]

    def verdict(self, **overrides: object) -> dict:
        kwargs = dict(
            audit_on=self.SATURATING,
            audit_off=self.CLEAN,
            spread=0.002,
            quota_moved=False,
            audit_rows_grew_on=True,
            audit_rows_grew_off=False,
        )
        kwargs.update(overrides)
        return harness.verdict(**kwargs)

    def test_a_knee_that_disappears_with_audit_off_passes(self) -> None:
        result = self.verdict()
        self.assertTrue(result["ok"], result["failures"])
        self.assertEqual(result["knee_audit_on"], 100.0)
        self.assertIsNone(result["knee_audit_off"])

    def test_quota_rejections_invalidate_the_run(self) -> None:
        # MUTANT: the admission limiter fired. The knee is then a property of the
        # limiter, and it lands at a rate low enough to look like a real ceiling.
        result = self.verdict(quota_moved=True)
        self.assertFalse(result["ok"])
        self.assertTrue(any("limiter" in f for f in result["failures"]))

    def test_audit_log_must_grow_while_audit_is_enabled(self) -> None:
        # MUTANT: the subject was never exercised, so the knee belongs to
        # something else entirely.
        result = self.verdict(audit_rows_grew_on=False)
        self.assertFalse(result["ok"])
        self.assertTrue(any("was not exercised" in f for f in result["failures"]))

    def test_audit_off_control_must_actually_be_off(self) -> None:
        # MUTANT: BUZZ_AUDIT_ENABLED did not take effect. The control then agrees
        # with the hypothesis for the wrong reason.
        result = self.verdict(audit_rows_grew_off=True)
        self.assertFalse(result["ok"])
        self.assertTrue(any("control did not take effect" in f for f in result["failures"]))

    def test_a_knee_that_survives_audit_off_fails(self) -> None:
        # MUTANT: same knee with audit disabled, so the audit path is not what
        # limits ingest and finding 1 does not explain the ceiling.
        result = self.verdict(audit_off=self.SATURATING)
        self.assertFalse(result["ok"])
        self.assertTrue(any("knee did not move" in f for f in result["failures"]))

    def test_no_knee_at_all_is_reported_as_a_violation(self) -> None:
        # Not a defect in the harness: a sweep that never saturates refutes the
        # predicted ceiling at these rates, and that has to be loud.
        result = self.verdict(audit_on=self.CLEAN)
        self.assertFalse(result["ok"])
        self.assertTrue(any("not the ingest ceiling" in f for f in result["failures"]))

    def test_every_failure_is_reported_not_just_the_first(self) -> None:
        result = self.verdict(quota_moved=True, audit_rows_grew_on=False)
        self.assertEqual(len(result["failures"]), 2)

    def test_measured_noise_widens_what_counts_as_a_knee(self) -> None:
        # At a 6% spread the 0.80 point sits above the threshold, so the knee
        # moves up the sweep instead of being asserted by a fixed constant.
        result = self.verdict(spread=0.07)
        self.assertEqual(result["knee_audit_on"], 200.0)

    def test_the_lock_ceiling_is_never_reported_as_absent(self) -> None:
        # The worker ceiling is lower and masks the lock ceiling, so a passing run
        # says nothing about the lock. This wording is the guard against a later
        # reader quoting the run as evidence the lock is fine.
        self.assertIn("structurally blind", self.verdict()["lock_ceiling"])


class ModelTests(unittest.TestCase):
    def test_the_documented_model_satisfies_its_own_contract(self) -> None:
        report = harness.model()
        self.assertTrue(report["verdict"]["ok"], report["verdict"]["failures"])

    def test_the_model_brackets_the_serialized_audit_ceiling(self) -> None:
        # 6 round trips at ~0.5ms is ~3ms per entry behind one worker, so ~333/s.
        lower, upper = harness.model()["verdict"]["ceiling_bracket_audit_on"]
        self.assertLess(lower, 1000.0 / 3.0)
        self.assertGreater(upper, 1000.0 / 3.0)


class ConnectionSizingTests(unittest.TestCase):
    def test_connection_count_scales_with_offered_rate(self) -> None:
        self.assertEqual(harness.conns_for(20.0), harness.MIN_CONNS)
        self.assertEqual(harness.conns_for(400.0), 16)

    def test_the_generator_is_never_the_narrower_pipe(self) -> None:
        # Each connection is closed-loop: one send per service latency. At a
        # pessimistic 20ms that is 50/s per connection, and the sizing has to
        # leave the offer reachable or the sweep measures the generator.
        for rate in (20.0, 50.0, 100.0, 200.0, 400.0):
            self.assertGreater(harness.conns_for(rate) * 50.0, rate)


if __name__ == "__main__":
    unittest.main()
