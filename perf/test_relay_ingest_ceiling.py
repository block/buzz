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
        "duration_secs": 20,
        "counters_window_aligned": True,
        "setup_overhead_fraction": 0.01,
        "generator_headroom": 0.56,
        "generator_lag_p99_ms": 0.3,
        "attempted_over_offered": 0.56,
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

    def test_a_starved_generator_disqualifies_its_cell(self) -> None:
        # MUTANT for the replacement gate.
        problems = harness.cell_problems(cell(generator_lag_p99_ms=45.0))
        self.assertTrue(any("free to send" in p for p in problems), problems)

    def test_a_healthy_generator_at_a_saturated_rate_passes(self) -> None:
        # A saturated cell has a poor apparent capacity and a poor attempted
        # fraction, and is still good evidence.
        self.assertEqual(
            harness.cell_problems(
                cell(
                    generator_lag_p99_ms=0.4,
                    generator_headroom=0.56,
                    attempted_over_offered=0.56,
                )
            ),
            [],
        )

    def test_headroom_is_reported_not_gated(self) -> None:
        # Issuability is established by the control arm holding its offer, not by
        # a per-cell margin the saturated arm can never clear.
        self.assertEqual(harness.cell_problems(cell(generator_headroom=1.1)), [])
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
        self.assertTrue(any("never compared" in f for f in result["failures"]))

    def test_lost_evidence_reports_inconclusive_not_negative(self) -> None:
        # Two rates: one comparable and unseparated, one that lost its evidence.
        # Reporting "not shown to limit ingest" here would state a result about
        # the relay when the truth is that the informative cells were dropped.
        on = arm([0.60, 0.62, 0.61], rate=100.0) + arm([0.60, 0.62], rate=800.0)
        on[-1]["outstanding_delta"] = 1000
        off = arm([0.61, 0.60, 0.62], audit=False, rate=100.0) + arm(
            [1.0, 1.0], audit=False, rate=800.0
        )
        result = harness.verdict(on, off, control_ran=True)
        self.assertFalse(result["ok"])
        self.assertTrue(
            any("inconclusive rather than negative" in f for f in result["failures"]),
            result["failures"],
        )

    def test_a_fatal_problem_is_not_also_listed_as_an_exclusion(self) -> None:
        on = arm([0.60, 0.62, 0.58, 0.61])
        on[0]["audit_send_errors_delta"] = 1
        result = harness.verdict(on, arm([1.0, 1.0, 0.999, 1.0], audit=False), True)
        self.assertTrue(any("worker is gone" in f for f in result["failures"]))
        self.assertFalse(
            any(
                "worker is gone" in r
                for c in result["excluded_cells"]
                for r in c["reasons"]
            )
        )

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


class CausalValidityTests(unittest.TestCase):
    """The gates that stop a precise answer to the wrong experiment."""

    def test_both_arms_collapsing_is_not_a_positive_control(self) -> None:
        # MUTANT, and the shipped bug: audit-on ~0.40 against audit-off ~0.50
        # separates cleanly and means nothing. Removing the audit path has to
        # restore the offer, not merely do better than keeping it.
        on = arm([0.40, 0.41, 0.39, 0.40])
        off = arm([0.50, 0.51, 0.49, 0.50], audit=False)
        result = harness.verdict(on, off, control_ran=True)
        self.assertFalse(result["ok"])
        self.assertTrue(
            any("did not hold its offer" in f for f in result["failures"]),
            result["failures"],
        )

    def test_a_control_that_holds_its_offer_passes(self) -> None:
        result = harness.verdict(
            arm([0.60, 0.62, 0.58, 0.61]),
            arm([1.0, 0.999, 1.0, 0.998], audit=False),
            control_ran=True,
        )
        self.assertTrue(result["ok"], result["failures"])
        self.assertTrue(
            result["arm_separation"]["primary_control_holds_offer"]
        )

    def test_only_secondary_rates_separating_does_not_pass(self) -> None:
        # MUTANT for the familywise hole: passing on any-of-N rates gives a ~10%
        # false-pass rate at five rates, measured on this module. One predeclared
        # primary contrast is the fix, so a separation at a lower rate while the
        # primary does not separate must fail.
        on = arm([0.60, 0.62, 0.58, 0.61], rate=800.0) + arm(
            [1.0, 0.999, 1.0, 0.998], rate=1600.0
        )
        off = arm([1.0, 0.999, 1.0, 0.998], audit=False, rate=800.0) + arm(
            [1.0, 0.999, 1.0, 0.998], audit=False, rate=1600.0
        )
        result = harness.verdict(on, off, control_ran=True)
        self.assertFalse(result["ok"])
        self.assertTrue(
            any("only secondary rates separated" in f for f in result["failures"]),
            result["failures"],
        )

    def test_a_reverse_separation_contradicts_rather_than_fails_to_support(self) -> None:
        on = arm([1.0, 0.999, 1.0, 0.998], rate=800.0)
        off = arm([0.60, 0.62, 0.58, 0.61], audit=False, rate=800.0)
        result = harness.verdict(on, off, control_ran=True)
        self.assertFalse(result["ok"])
        self.assertTrue(
            any("contradicts the hypothesis" in f for f in result["failures"]),
            result["failures"],
        )

    def test_an_auditing_control_arm_is_not_a_control(self) -> None:
        # MUTANT: the rig JSON claiming audit is off is a claim about how the
        # relay was started, and under --skip-relay nobody verified it.
        off = arm([1.0, 0.999, 1.0, 0.998], audit=False)
        off[0]["audit_activity_in_control_arm"] = 4105
        result = harness.verdict(arm([0.60, 0.62, 0.58, 0.61]), off, True)
        self.assertTrue(
            any(
                "still auditing" in r
                for c in result["excluded_cells"]
                for r in c["reasons"]
            ),
            result["excluded_cells"],
        )

    def test_unaligned_counter_windows_disqualify_a_cell(self) -> None:
        # MUTANT: counters bracketing the whole subprocess while rates divide by
        # the post-connect window can hide banking and push busy above 1.0.
        problems = harness.cell_problems(cell(counters_window_aligned=False))
        self.assertTrue(any("timed window" in p for p in problems))

    def test_aligned_counter_windows_pass(self) -> None:
        self.assertEqual(harness.cell_problems(cell(counters_window_aligned=True)), [])

    def test_heavy_setup_overhead_disqualifies_a_cell(self) -> None:
        problems = harness.cell_problems(cell(setup_overhead_fraction=0.20))
        self.assertTrue(any("setup and teardown" in p for p in problems))

    def test_closed_loop_shortfall_does_not_disqualify_a_saturated_cell(self) -> None:
        # MUTANT for a gate that was added and then had to be removed: at a
        # saturated rate a closed-loop sender can reach neither its scheduled
        # slots nor a capacity margin, so gating on either rejected every cell
        # that matters.
        problems = harness.cell_problems(
            cell(attempted_over_offered=0.57, generator_headroom=0.59)
        )
        self.assertEqual(problems, [])


class GeneratorEnvironmentTests(unittest.TestCase):
    RIG = {
        "bench_private_key": "ab" * 32,
        "metrics_url": "http://localhost:9202/metrics",
    }

    def test_the_metrics_url_reaches_the_generator(self) -> None:
        # Its absence excludes the whole run, and nothing else in the suite can
        # see that: these tests build cells directly and the model bypasses
        # run_cell. See `generator_env`.
        env = harness.generator_env(self.RIG)
        self.assertEqual(env["BENCH_METRICS_URL"], self.RIG["metrics_url"])
        self.assertEqual(env["BENCH_PRIVATE_KEY"], self.RIG["bench_private_key"])

    def test_inherited_cli_credentials_are_scrubbed(self) -> None:
        # A stale BUZZ_AUTH_TAG fails the dev relay's first write outright.
        import os as _os

        for name in ("BUZZ_AUTH_TAG", "BUZZ_RELAY_URL", "BUZZ_PRIVATE_KEY"):
            _os.environ[name] = "stale"
        try:
            env = harness.generator_env(self.RIG)
            for name in ("BUZZ_AUTH_TAG", "BUZZ_RELAY_URL", "BUZZ_PRIVATE_KEY"):
                self.assertNotIn(name, env)
        finally:
            for name in ("BUZZ_AUTH_TAG", "BUZZ_RELAY_URL", "BUZZ_PRIVATE_KEY"):
                _os.environ.pop(name, None)


class WorkerRateRegimeTests(unittest.TestCase):
    def test_estimates_are_reported_per_rate_not_pooled(self) -> None:
        # Different offered rates are different load and database regimes, not
        # repeats of one estimand.
        cells = [
            cell(offered_per_s=800.0, audit_completed_per_s=450.0),
            cell(offered_per_s=800.0, audit_completed_per_s=460.0),
            cell(offered_per_s=1600.0, audit_completed_per_s=400.0),
            cell(offered_per_s=1600.0, audit_completed_per_s=410.0),
        ]
        per_rate = harness.worker_rate(cells)["per_rate"]
        self.assertEqual([r["offered_per_s"] for r in per_rate], [800.0, 1600.0])
        self.assertAlmostEqual(per_rate[0]["completed_per_s"]["mean"], 455.0)
        self.assertAlmostEqual(per_rate[1]["completed_per_s"]["mean"], 405.0)


class TTableTests(unittest.TestCase):
    def test_rounding_errs_wide_not_narrow(self) -> None:
        # The sparse table must never return a smaller critical value than the
        # true one: df 11 is 2.201 and df 13 is 2.160, and picking the next higher
        # stored df would under-cover at exactly the n>=16 cells the plan specs.
        self.assertGreaterEqual(harness.t95(11), 2.201)
        self.assertGreaterEqual(harness.t95(13), 2.160)
        self.assertGreaterEqual(harness.t95(14), 2.145)

    def test_exact_entries_are_returned_unchanged(self) -> None:
        self.assertEqual(harness.t95(4), 2.776)
        self.assertEqual(harness.t95(10), 2.228)


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
            "source_diff_digest": "clean",
            "binary_digest": "cafe",
            "database_reset": True,
        }
        base.update(identity)
        fractions = [0.60, 0.62, 0.58, 0.61] if audit else [1.0, 0.999, 1.0, 0.998]
        cells = []
        for rate in base["rates"]:
            for f in fractions[: base["repeats"]]:
                cells.extend(arm([f], audit=audit, rate=rate))
        return {"audit_enabled": audit, "identity": base, "cells": cells}

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

    def test_cells_must_cover_the_declared_rate_grid(self) -> None:
        # MUTANT, probe-confirmed by the reviewers: an identity declaring
        # [800, 1600] with only 800-cells supplied used to return ok.
        broken = self.half(True, rates=[800.0, 1600.0])
        broken["cells"] = [c for c in broken["cells"] if c["offered_per_s"] == 800.0]
        with self.assertRaises(ValueError) as ctx:
            harness.combine(broken, self.half(False, rates=[800.0, 1600.0]))
        self.assertIn("rate grid", str(ctx.exception))

    def test_cells_must_carry_the_declared_repeat_count(self) -> None:
        broken = self.half(True)
        broken["cells"] = broken["cells"][:-1]
        with self.assertRaises(ValueError) as ctx:
            harness.combine(broken, self.half(False))
        self.assertIn("repeats", str(ctx.exception))

    def test_a_cell_labelled_for_the_other_arm_is_rejected(self) -> None:
        # MUTANT: audit-off-labelled cells inside the audit-on report used to pass.
        broken = self.half(True)
        broken["cells"] = arm([0.6, 0.62, 0.58, 0.61], audit=False)
        with self.assertRaises(ValueError) as ctx:
            harness.combine(broken, self.half(False))
        self.assertIn("audit_enabled", str(ctx.exception))

    def test_a_dirty_tree_digest_mismatch_is_rejected(self) -> None:
        with self.assertRaises(ValueError):
            harness.combine(
                self.half(True), self.half(False, source_diff_digest="beef")
            )

    def test_a_half_that_skipped_the_reset_is_rejected(self) -> None:
        # MUTANT, probe-confirmed: two halves that both skipped the reset agree,
        # and equality passed them. Equality is not truth.
        with self.assertRaises(ValueError) as ctx:
            harness.combine(
                self.half(True, database_reset=False),
                self.half(False, database_reset=False),
            )
        self.assertIn("database snapshot", str(ctx.exception))

    def test_an_incomplete_cell_is_rejected(self) -> None:
        # MUTANT: an absent validity field skips its gate, so a cell missing
        # `counters_window_aligned` read as valid evidence.
        broken = self.half(True)
        del broken["cells"][0]["counters_window_aligned"]
        with self.assertRaises(ValueError) as ctx:
            harness.combine(broken, self.half(False))
        self.assertIn("counters_window_aligned", str(ctx.exception))

    def test_a_cell_without_a_duration_is_rejected(self) -> None:
        broken = self.half(True)
        del broken["cells"][0]["duration_secs"]
        with self.assertRaises(ValueError):
            harness.combine(broken, self.half(False))

    def test_a_binary_digest_mismatch_is_rejected(self) -> None:
        # Two builds at one commit are two experiments; the source digest is
        # taken after the build and cannot prove what actually ran.
        with self.assertRaises(ValueError):
            harness.combine(self.half(True), self.half(False, binary_digest="beef"))

    def test_arms_reset_differently_are_rejected(self) -> None:
        # A fixed arm order against a database that grew in between confounds arm
        # with time and index size; restoring the same snapshot at both boundaries
        # is what makes them comparable, so a pair where only one arm was reset
        # is not one experiment.
        with self.assertRaises(ValueError) as ctx:
            harness.combine(self.half(True), self.half(False, database_reset=False))
        self.assertIn("database_reset", str(ctx.exception))

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
        self.assertEqual(rates, [800.0, 1600.0])

    def test_the_default_grid_straddles_the_measured_drain_band(self) -> None:
        # The regression guard for a grid that cannot fire the contract. A top
        # rate at the drain rate makes the only informative cell a coin flip, and
        # the run then reports "not shown to limit ingest" — a false negative
        # phrased as a conclusion.
        band = harness.MEASURED_DRAIN_BAND_PER_S
        self.assertLess(min(harness.DEFAULT_RATES), band)
        self.assertGreaterEqual(max(harness.DEFAULT_RATES), 2.0 * band)

    def test_the_model_ceiling_is_not_below_its_own_grid_by_construction(self) -> None:
        # The earlier model fixed its ceiling at 333/s under a 400/s top rate, so
        # it separated at 120% utilization while the rig sat at 81-102%. Green
        # then described the fixture rather than the contract.
        self.assertGreater(
            harness.MEASURED_DRAIN_BAND_PER_S, max(harness.DEFAULT_RATES) / 8.0
        )
        self.assertLess(harness.MEASURED_DRAIN_BAND_PER_S, max(harness.DEFAULT_RATES))

    def test_a_ceiling_above_the_grid_reports_a_negative_not_a_pass(self) -> None:
        # MUTANT: nothing saturates, so there is nothing to separate. The run must
        # fail, and must say it compared every rate and found nothing rather than
        # implying the evidence was missing.
        verdict = harness.model(ceiling_per_s=2.0 * max(harness.DEFAULT_RATES))["verdict"]
        self.assertFalse(verdict["ok"])
        # And the diagnosis is the saturation one, not the exoneration one: a
        # ceiling above the grid means the grid never reached the audit path, so
        # "none separated" would be the wrong thing for a reader to act on.
        self.assertTrue(
            any("never saturated" in f for f in verdict["failures"]), verdict["failures"]
        )
        self.assertFalse(verdict["saturation"]["any_rate_saturated"])

    def test_a_transition_rate_fills_the_channel_over_several_repeats(self) -> None:
        # Partial banking, as distinct from the deep +1000-from-empty shape: an
        # offer just above the drain rate accumulates across repeats until the
        # channel caps.
        verdict = harness.model(ceiling_per_s=450.0, rates=[100.0, 470.0])["verdict"]
        deltas = [c["offered_per_s"] for c in verdict["excluded_cells"]]
        self.assertTrue(deltas)
        self.assertTrue(all(r == 470.0 for r in deltas))
        self.assertGreater(len(deltas), 1)

    def test_the_model_derives_its_generator_metrics(self) -> None:
        # Fixturing these is how two gates built on them stayed invisible: an
        # absent key skips a gate, and a hard-coded healthy value passes it. The
        # model must produce the saturated regime's real shape.
        on = harness.model()["verdict"]
        saturated = [
            r for r in on["arm_separation"]["by_rate"] if r.get("separated_here")
        ]
        self.assertTrue(saturated)
        # And the derived headroom at a saturated rate is below 1, which the old
        # gate would have rejected.
        cells = harness.model(ceiling_per_s=450.0, rates=[1600.0])
        self.assertTrue(cells["verdict"]["saturation"]["any_rate_saturated"])

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
        self.assertEqual(separation["comparable_rates"], len(harness.DEFAULT_RATES))

    def test_the_model_reports_the_serialized_worker_rate(self) -> None:
        # The worker rate the model recovers is the ceiling it was given, which is
        # now the drain rate measured on this rig rather than a round number
        # chosen to sit below the grid.
        estimate = harness.model()["verdict"]["worker_rate"]["estimate"]
        self.assertAlmostEqual(estimate["mean"], harness.MEASURED_DRAIN_BAND_PER_S, 6)


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
        self.assertEqual(harness.conns_for(800.0), 32)

    def test_sizing_lets_an_unsaturated_cell_meet_its_offer(self) -> None:
        # That is all the sizing has to buy. Above the ceiling the relay sets the
        # pace and more connections only lengthen the queue, so the target is the
        # unsaturated service time, not the saturated one.
        unsaturated_service_s = 0.008
        for rate in harness.DEFAULT_RATES:
            self.assertGreater(harness.conns_for(rate) / unsaturated_service_s, rate)


if __name__ == "__main__":
    unittest.main()
