#!/usr/bin/env python3
"""Unit tests for the relay bus scaling harness."""

from __future__ import annotations

import argparse
import unittest

import relay_bus_scaling as harness


class RelayBusScalingTests(unittest.TestCase):
    def args(self, **overrides: object) -> argparse.Namespace:
        defaults = dict(
            mode="model",
            pods="1,2,4",
            communities=64,
            events_per_community_per_sec=100.0,
            subscribed_communities=1,
            interested_pods_per_subscribed_community=0,
            redis_url="redis://127.0.0.1:6379/0",
            redis_timeout=1.0,
            assert_scaling=True,
            min_reduction_ratio=0.95,
            max_scoped_irrelevant_pct=0.0,
        )
        defaults.update(overrides)
        return argparse.Namespace(**defaults)

    def test_scoped_global_channel_matches_relay_topic_shape(self) -> None:
        community = "00000000-0000-0000-0000-000000000001"
        self.assertEqual(
            harness.scoped_global_channel(community),
            "buzz:00000000-0000-0000-0000-000000000001:global",
        )

    def test_model_default_proves_sixty_four_x_reduction_for_one_two_four_pods(self) -> None:
        args = self.args()
        rows = harness.model_measurements(args, [1, 2, 4])

        self.assertEqual([row.pods for row in rows], [1, 2, 4])
        self.assertEqual([row.reduction for row in rows], [64.0, 64.0, 64.0])
        self.assertEqual([row.new_irrelevant_pct for row in rows], [0.0, 0.0, 0.0])
        harness.assert_scaling(args, rows)

    def test_assertion_fails_when_scoped_bus_receives_irrelevant_messages(self) -> None:
        args = self.args()
        mutant_row = harness.Measurement(
            pods=4,
            old_cluster=25_600,
            old_avg_pod=6_400,
            old_irrelevant_pct=98.44,
            new_cluster=25_600,
            new_avg_pod=6_400,
            new_irrelevant_pct=98.44,
            reduction=1.0,
            published=6_400,
        )

        with self.assertRaises(AssertionError):
            harness.assert_scaling(args, [mutant_row])


if __name__ == "__main__":
    unittest.main()
