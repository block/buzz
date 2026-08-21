#!/usr/bin/env python3
"""Unit tests for the ingest-ceiling harness verdict logic.

Every test that asserts a *pass* is paired with a mutant that must fail. A
harness whose contract cannot go red is decoration.
"""

from __future__ import annotations

import unittest

import relay_ingest_ceiling as harness


def cell(**overrides) -> dict:
    """A valid, steady, busy audit-on cell. Mutants override one field."""
    base = {
        "offered_per_s": 800.0,
        "audit_enabled": True,
        "accepted_over_offered": 0.60,
        "accepted_per_s": 480.0,
        "rejected": 0,
        "transport_errors": 0,
        "quota_rejections_delta": 0,
        "unavailable_rejections_delta": 0,
        "audit_log_errors_delta": 0,
        "audit_send_errors_delta": 0,
        "generator_headroom": 4.0,
        "audit_completed_per_s": 450.0,
        "audit_service_mean_ms": 2.2,
        "audit_busy_fraction": 0.99,
        "outstanding_delta": 0,
    }
    base.update(overrides)
    return base


def arm(fractions: list[float], audit: bool = True, rate: float = 800.0) -> list:
    return [
        cell(
            offered_per_s=rate,
            accepted_over_offered=f,
            audit_enabled=audit,
            audit_completed_per_s=450.0 if audit else None,
            audit_service_mean_ms=2.2 if audit else None,
            audit_busy_fraction=0.99 if audit else None,
            outstanding_delta=0 if audit else None,
        )
        for f in fractions
    ]


class StatisticsTests(unittest.TestCase):
    def test_sample_stddev_is_n_minus_one(self) -> None:
        self.assertAlmostEqual(harness.sample_stddev([2.0, 4.0, 4.0, 4.0, 5.0]), 1.0954451, 6)

    def test_stddev_needs_two_observations(self) -> None:
        with self.assertRaises(ValueError):
            harness.sample_stddev([1.0])

    def test_stddev_does_not_grow_with_n_the_way_range_does(self) -> None:
        # MUTANT guard for the estimator that was replaced: (max-min)/mean rises
        # with sample size on its own, so the same population read at n=2 and
        # n=6 would report different spreads. Stddev does not.
        small = [10.0, 12.0]
        large = [10.0, 10.4, 10.8, 11.2, 11.6, 12.0]
        range_small = (max(small) - min(small)) / harness.mean(small)
        range_large = (max(large) - min(large)) / harness.mean(large)
        self.assertAlmostEqual(range_small, range_large, 6)  # range is blind here
        self.assertLess(harness.sample_stddev(large), harness.sample_stddev(small))

    def test_interval_is_none_bounded_for_a_single_observation(self) -> None:
        self.assertEqual(harness.confidence_interval([5.0])["lo"], None)

    def test_difference_interval_excludes_zero_when_arms_differ(self) -> None:
        d = harness.difference_interval([1.0, 1.0, 0.999, 1.0], [0.6, 0.61, 0.59, 0.6])
        self.assertTrue(d["excludes_zero"])
        self.assertGreater(d["diff"], 0.0)

    def test_difference_interval_spans_zero_when_arms_agree(self) -> None:
        # MUTANT: same population twice. A predicate built on "the intervals do
        # not overlap" would be weaker; one built on "they overlap, so the arms
        # are equal" would be invalid. Neither is what this returns.
        d = harness.difference_interval([0.60, 0.62, 0.58, 0.61], [0.61, 0.59, 0.60, 0.62])
        self.assertFalse(d["excludes_zero"])


class CellValidityTests(unittest.TestCase):
    def test_a_clean_cell_has_no_problems(self) -> None:
        self.assertEqual(harness.cell_problems(cell()), [])

    def test_quota_rejections_invalidate(self) -> None:
        self.assertTrue(
            any("limiter" in p for p in harness.cell_problems(cell(quota_rejections_delta=2)))
        )

    def test_unavailable_rejections_invalidate(self) -> None:
        # MUTANT for the hole the first version had: `unavailable` takes the same
        # NOTICE-without-OK path as quota and is load-correlated, so it can forge
        # a knee that survives repeats.
        problems = harness.cell_problems(cell(unavailable_rejections_delta=1))
        self.assertTrue(any("load-correlated" in p for p in problems))

    def test_audit_send_failure_invalidates(self) -> None:
        problems = harness.cell_problems(cell(audit_send_errors_delta=1))
        self.assertTrue(any("worker is gone" in p for p in problems))

    def test_relay_rejections_and_transport_errors_invalidate(self) -> None:
        self.assertTrue(harness.cell_problems(cell(rejected=3)))
        self.assertTrue(harness.cell_problems(cell(transport_errors=1)))

    def test_thin_generator_headroom_invalidates(self) -> None:
        problems = harness.cell_problems(cell(generator_headroom=1.1))
        self.assertTrue(any("measuring the generator" in p for p in problems))

    def test_ample_headroom_passes(self) -> None:
        self.assertEqual(harness.cell_problems(cell(generator_headroom=2.0)), [])


class SteadyStateTests(unittest.TestCase):
    def test_level_outstanding_work_is_steady(self) -> None:
        self.assertTrue(harness.steady_state(cell(outstanding_delta=0)))

    def test_a_full_queue_start_and_end_is_steady(self) -> None:
        # The saturated regime settles with the channel full. A gate written as
        # "the queue must be empty at the start" would delete every cell that
        # matters for a ceiling; this one accepts any stable level.
        self.assertTrue(harness.steady_state(cell(outstanding_delta=10)))

    def test_banking_the_whole_channel_is_not_steady(self) -> None:
        # MUTANT: +1000 is exactly the measured empty-start acceptance credit.
        self.assertFalse(harness.steady_state(cell(outstanding_delta=1000)))

    def test_audit_off_cells_have_no_steady_state_reading(self) -> None:
        self.assertIsNone(harness.steady_state(cell(outstanding_delta=None)))

    def test_an_audit_off_cell_is_still_evidence(self) -> None:
        # No queue means no steady-state reading, which must not be mistaken for
        # failing the check - otherwise the whole control arm drops out.
        self.assertTrue(harness.cell_is_evidence(cell(outstanding_delta=None)))

    def test_a_banking_cell_is_not_evidence(self) -> None:
        self.assertFalse(harness.cell_is_evidence(cell(outstanding_delta=1000)))


class ArmSeparationTests(unittest.TestCase):
    def test_separation_is_found_where_the_arms_diverge(self) -> None:
        result = harness.arm_separation(
            arm([0.60, 0.62, 0.58, 0.61]), arm([1.0, 1.0, 0.999, 1.0], audit=False)
        )
        self.assertTrue(result["separated"])

    def test_identical_arms_are_not_separated(self) -> None:
        # MUTANT: the control changed nothing, so the audit path is not shown to
        # limit ingest and the run must not pass.
        result = harness.arm_separation(
            arm([0.60, 0.62, 0.58, 0.61]), arm([0.61, 0.59, 0.60, 0.62], audit=False)
        )
        self.assertFalse(result["separated"])

    def test_separation_in_the_wrong_direction_does_not_count(self) -> None:
        # MUTANT: audit-off *worse* than audit-on refutes the hypothesis; it must
        # not satisfy a predicate that only looks at "the interval excludes zero".
        result = harness.arm_separation(
            arm([1.0, 1.0, 0.999, 1.0]), arm([0.60, 0.62, 0.58, 0.61], audit=False)
        )
        self.assertFalse(result["separated"])


class WorkerRateTests(unittest.TestCase):
    def test_busy_steady_cells_give_an_estimate(self) -> None:
        result = harness.worker_rate([cell(), cell(audit_completed_per_s=460.0)])
        self.assertEqual(result["cells"], 2)
        self.assertAlmostEqual(result["estimate"]["mean"], 455.0)

    def test_an_idle_worker_cannot_report_capacity(self) -> None:
        # MUTANT: below saturation the completion rate tracks the offer, so it is
        # not the worker's limit.
        result = harness.worker_rate([cell(audit_busy_fraction=0.40)])
        self.assertEqual(result["cells"], 0)
        self.assertIsNone(result["estimate"])

    def test_a_cell_banking_the_channel_cannot_report_capacity(self) -> None:
        result = harness.worker_rate([cell(outstanding_delta=1000)])
        self.assertEqual(result["cells"], 0)

    def test_an_invalid_cell_cannot_report_capacity(self) -> None:
        result = harness.worker_rate([cell(unavailable_rejections_delta=1)])
        self.assertEqual(result["cells"], 0)


class VerdictTests(unittest.TestCase):
    def passing(self) -> dict:
        return harness.verdict(
            arm([0.60, 0.62, 0.58, 0.61]),
            arm([1.0, 1.0, 0.999, 1.0], audit=False),
            control_ran=True,
        )

    def test_separated_arms_with_clean_cells_pass(self) -> None:
        result = self.passing()
        self.assertTrue(result["ok"], result["failures"])
        self.assertTrue(result["control"]["ran"])

    def test_a_missing_control_cannot_pass(self) -> None:
        # MUTANT: this is the --skip-audit-off shape. Attribution was never
        # tested, so no verdict is available at any cell quality.
        result = harness.verdict(arm([0.60, 0.62, 0.58, 0.61]), None, control_ran=False)
        self.assertFalse(result["ok"])
        self.assertFalse(result["control"]["ran"])
        self.assertTrue(any("partial experiment" in f for f in result["failures"]))

    def test_the_report_says_whether_the_control_ran(self) -> None:
        # "no knee on the audit-off arm" used to read identically for "the
        # control ran and the knee was gone" and "the control never ran".
        self.assertTrue(self.passing()["control"]["ran"])
        self.assertFalse(
            harness.verdict(arm([0.6, 0.61]), None, control_ran=False)["control"]["ran"]
        )

    def test_an_unsteady_cell_is_excluded_not_fatal(self) -> None:
        # The first repeat of the first saturating rate starts with the audit
        # channel empty and banks its whole depth, so failing the run on a
        # non-steady cell would fail every genuine saturating dataset. It is
        # dropped from the evidence and reported instead.
        on = arm([0.60, 0.62, 0.58, 0.61])
        on[0]["outstanding_delta"] = 1000
        on[0]["accepted_over_offered"] = 0.73  # the credit inflates this
        result = harness.verdict(on, arm([1.0, 1.0, 0.999, 1.0], audit=False), True)
        self.assertTrue(result["ok"], result["failures"])
        self.assertEqual(len(result["excluded_cells"]), 1)
        self.assertTrue(
            any("acceptance credit" in r for r in result["excluded_cells"][0]["reasons"])
        )

    def test_an_excluded_cell_is_kept_out_of_the_interval(self) -> None:
        # Not only must it not fail the run, its inflated accepted/offered must
        # not widen or shift the arm's interval.
        on = arm([0.60, 0.62, 0.58, 0.61])
        on[0]["outstanding_delta"] = 1000
        on[0]["accepted_over_offered"] = 0.73
        result = harness.verdict(on, arm([1.0, 1.0, 0.999, 1.0], audit=False), True)
        rate = result["arm_separation"]["by_rate"][0]
        self.assertEqual(rate["evidence_cells"]["audit_on"], 3)
        self.assertEqual(rate["dropped_cells"], 1)
        self.assertLess(rate["audit_on"]["mean"], 0.63)

    def test_a_dead_audit_worker_is_fatal_not_merely_excluded(self) -> None:
        # An enqueue failure means the receiver is gone, so every later cell is
        # suspect - not just the one that noticed.
        on = arm([0.60, 0.62, 0.58, 0.61])
        on[1]["audit_send_errors_delta"] = 1
        result = harness.verdict(on, arm([1.0, 1.0, 0.999, 1.0], audit=False), True)
        self.assertFalse(result["ok"])
        self.assertTrue(any("worker is gone" in f for f in result["failures"]))

    def test_too_few_surviving_cells_fails_the_run(self) -> None:
        # MUTANT: exclusion must not become a way to pass by discarding almost
        # everything. One evidence cell per arm cannot support an interval.
        on = arm([0.60, 0.62])
        on[0]["outstanding_delta"] = 1000
        off = arm([1.0, 1.0], audit=False)
        result = harness.verdict(on, off, control_ran=True)
        self.assertFalse(result["ok"])
        self.assertTrue(any("cannot be compared" in f for f in result["failures"]))

    def test_no_separation_fails_the_run(self) -> None:
        result = harness.verdict(
            arm([0.60, 0.62, 0.58, 0.61]),
            arm([0.61, 0.59, 0.60, 0.62], audit=False),
            control_ran=True,
        )
        self.assertFalse(result["ok"])
        self.assertTrue(any("not shown to limit ingest" in f for f in result["failures"]))

    def test_every_failure_is_reported_not_just_the_first(self) -> None:
        on = arm([0.60, 0.62, 0.58, 0.61])
        on[0]["audit_send_errors_delta"] = 1
        result = harness.verdict(on, None, control_ran=False)
        self.assertGreaterEqual(len(result["failures"]), 2)
        self.assertTrue(any("worker is gone" in f for f in result["failures"]))
        self.assertTrue(any("partial experiment" in f for f in result["failures"]))

    def test_exclusions_and_failures_are_reported_separately(self) -> None:
        # A contaminated cell and a banking cell are both excluded with reasons;
        # neither is silently dropped, and neither is confused with a failure.
        on = arm([0.60, 0.62, 0.58, 0.61])
        on[0]["quota_rejections_delta"] = 1
        on[1]["outstanding_delta"] = 1000
        result = harness.verdict(on, arm([1.0, 1.0, 0.999, 1.0], audit=False), True)
        self.assertEqual(len(result["excluded_cells"]), 2)
        self.assertEqual(result["arm_separation"]["by_rate"][0]["dropped_cells"], 2)

    def test_the_lock_ceiling_is_never_reported_as_absent(self) -> None:
        # The worker ceiling is lower and masks the lock, so a passing run says
        # nothing about the lock. This wording guards against a later reader
        # quoting the run as evidence the lock is fine.
        self.assertIn("structurally blind", self.passing()["lock_ceiling"])


class CombineTests(unittest.TestCase):
    def half(self, audit: bool, **identity) -> dict:
        base = {
            "rates": [800.0],
            "duration_secs": 20,
            "repeats": 4,
            "targets": ["a.localhost:3030", "b.localhost:3030"],
            "ws_events_per_sec_limit": 100000,
            "messages_per_min_limit": 6000000,
            "generator": "./target/ci/ingest_load",
            "source_revision": "deadbeef",
        }
        base.update(identity)
        fractions = [0.60, 0.62, 0.58, 0.61] if audit else [1.0, 1.0, 0.999, 1.0]
        return {
            "audit_enabled": audit,
            "identity": base,
            "cells": arm(fractions, audit=audit),
        }

    def test_matched_halves_combine_and_pass(self) -> None:
        report = harness.combine(self.half(True), self.half(False))
        self.assertTrue(report["verdict"]["ok"], report["verdict"]["failures"])

    def test_order_of_the_halves_does_not_matter(self) -> None:
        self.assertTrue(harness.combine(self.half(False), self.half(True))["verdict"]["ok"])

    def test_two_halves_from_the_same_arm_are_rejected(self) -> None:
        with self.assertRaises(ValueError):
            harness.combine(self.half(True), self.half(True))

    def test_a_duration_mismatch_is_rejected(self) -> None:
        # MUTANT: the shipped bug. A 10s three-rate on-report combined happily
        # with a 60s two-rate off-report and returned ok.
        with self.assertRaises(ValueError) as ctx:
            harness.combine(self.half(True), self.half(False, duration_secs=60))
        self.assertIn("duration_secs", str(ctx.exception))

    def test_a_rate_grid_mismatch_is_rejected(self) -> None:
        with self.assertRaises(ValueError) as ctx:
            harness.combine(self.half(True), self.half(False, rates=[100.0, 200.0]))
        self.assertIn("rates", str(ctx.exception))

    def test_a_limiter_or_revision_mismatch_is_rejected(self) -> None:
        for field, value in (
            ("ws_events_per_sec_limit", 10),
            ("source_revision", "cafe1234"),
            ("targets", ["a.localhost:3030"]),
        ):
            with self.assertRaises(ValueError):
                harness.combine(self.half(True), self.half(False, **{field: value}))

    def test_combine_signals_misuse_the_way_its_neighbours_do(self) -> None:
        # ValueError, not SystemExit: the guard on the path that produced the
        # shipped verdict has to be assertable from a test.
        with self.assertRaises(ValueError):
            harness.combine(self.half(True), self.half(True))


class ModelTests(unittest.TestCase):
    def test_the_documented_model_satisfies_its_own_contract(self) -> None:
        report = harness.model()
        self.assertTrue(report["verdict"]["ok"], report["verdict"]["failures"])

    def test_the_model_separates_only_where_it_saturates(self) -> None:
        rates = [
            r["offered_per_s"]
            for r in harness.model()["verdict"]["arm_separation"]["by_rate"]
            if r.get("separated_here")
        ]
        self.assertEqual(rates, [350.0, 400.0])

    def test_the_model_reproduces_sequential_channel_fill(self) -> None:
        # The green path has to be tested against the physics, not against a
        # dataset where every cell is conveniently steady. The first saturating
        # rate banks acceptance credit until the channel is full, so the model
        # must produce excluded cells and still pass.
        verdict = harness.model()["verdict"]
        self.assertTrue(verdict["ok"], verdict["failures"])
        self.assertGreater(len(verdict["excluded_cells"]), 0)
        self.assertTrue(
            all(
                any("acceptance credit" in r for r in c["reasons"])
                for c in verdict["excluded_cells"]
            )
        )

    def test_the_model_keeps_every_rate_comparable(self) -> None:
        separation = harness.model()["verdict"]["arm_separation"]
        self.assertEqual(separation["comparable_rates"], 6)

    def test_the_model_reports_the_serialized_worker_rate(self) -> None:
        estimate = harness.model()["verdict"]["worker_rate"]["estimate"]
        self.assertAlmostEqual(estimate["mean"], 1000.0 / 3.0, 6)


class KneeReportingTests(unittest.TestCase):
    def test_knee_is_the_first_persistent_shortfall(self) -> None:
        points = [(20.0, 1.0), (50.0, 1.0), (100.0, 0.80), (200.0, 0.41)]
        self.assertEqual(harness.find_knee(points, 0.99), 100.0)

    def test_a_lone_dip_is_not_a_knee(self) -> None:
        # MUTANT: one rate dips and the next recovers. Saturation is monotone, so
        # this is noise, and a harness that called it a knee would report a
        # ceiling that moves run to run.
        points = [(20.0, 1.0), (50.0, 0.90), (100.0, 1.0), (200.0, 1.0)]
        self.assertIsNone(harness.find_knee(points, 0.99))

    def test_bracket_spans_the_last_pass_and_the_knee(self) -> None:
        points = [(20.0, 1.0), (50.0, 1.0), (100.0, 0.80), (200.0, 0.41)]
        self.assertEqual(harness.knee_bracket(points, 0.99), (50.0, 100.0))

    def test_points_need_not_arrive_sorted(self) -> None:
        points = [(200.0, 0.41), (20.0, 1.0), (100.0, 0.80), (50.0, 1.0)]
        self.assertEqual(harness.find_knee(points, 0.99), 100.0)


class FormattingTests(unittest.TestCase):
    def test_absent_values_format_instead_of_aborting_the_sweep(self) -> None:
        # A null percentile (every connection died before its first settled send)
        # or a null audit series (audit-off arm) used to abort mid-sweep with a
        # TypeError, losing the report and the --json file.
        self.assertEqual(harness._fmt(None, "{:.2f}ms"), "n/a")
        self.assertEqual(harness._fmt(2.5, "{:.2f}ms"), "2.50ms")


class ConnectionSizingTests(unittest.TestCase):
    def test_connection_count_scales_with_offered_rate(self) -> None:
        self.assertEqual(harness.conns_for(20.0), harness.MIN_CONNS)
        self.assertEqual(harness.conns_for(400.0), 16)

    def test_sizing_leaves_the_offer_reachable(self) -> None:
        # Each connection is closed-loop: one send per service time. At a
        # pessimistic 20ms that is 50/s per connection, and the sizing has to
        # leave the offer reachable or the sweep measures the generator.
        for rate in harness.DEFAULT_RATES + [800.0, 1600.0]:
            self.assertGreater(harness.conns_for(rate) * 50.0, rate)


if __name__ == "__main__":
    unittest.main()
