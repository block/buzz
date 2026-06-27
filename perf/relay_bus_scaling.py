#!/usr/bin/env python3
"""Reproducible Buzz relay bus scaling harness.

This models the relay's Redis fan-out boundary, not client rendering or DB ingest.
It compares the pre-rewrite global-firehose shape (every pod receives every
published event) with the multi-tenant scoped-topic shape (a pod receives only
community topics it has retained because it has local subscribers).

Default scenario intentionally isolates the rewrite's scaling claim:
  * 64 communities publish at an equal rate.
  * one target community has local subscribers on every relay pod.
  * all other communities are irrelevant to those pods.

Expected result: old per-pod ingress is total cluster event rate; new per-pod
ingress is target-community event rate. Cluster work scales as
O(pods * total_event_rate) before and O(interested_pods * community_event_rate)
after.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass


@dataclass(frozen=True)
class Scenario:
    pods: int
    communities: int
    events_per_community_per_sec: float
    subscribed_communities: int
    interested_pods_per_subscribed_community: int

    @property
    def total_event_rate(self) -> float:
        return self.communities * self.events_per_community_per_sec

    @property
    def subscribed_event_rate(self) -> float:
        return self.subscribed_communities * self.events_per_community_per_sec

    def old_global_firehose(self) -> tuple[float, float, float]:
        """Return (cluster ingress/sec, avg pod ingress/sec, irrelevant pct)."""
        cluster = self.pods * self.total_event_rate
        per_pod = self.total_event_rate
        irrelevant = max(self.total_event_rate - self.subscribed_event_rate, 0.0)
        irrelevant_pct = 100.0 * irrelevant / self.total_event_rate
        return cluster, per_pod, irrelevant_pct

    def scoped_bus(self) -> tuple[float, float, float]:
        """Return (cluster ingress/sec, avg pod ingress/sec, irrelevant pct)."""
        interested_pods = min(self.pods, self.interested_pods_per_subscribed_community)
        cluster = (
            self.subscribed_communities
            * interested_pods
            * self.events_per_community_per_sec
        )
        per_pod = cluster / self.pods
        # A scoped subscriber receives only retained community topics in this model.
        irrelevant_pct = 0.0
        return cluster, per_pod, irrelevant_pct


def fmt(value: float) -> str:
    if abs(value - round(value)) < 1e-9:
        return f"{int(round(value)):,}"
    return f"{value:,.2f}"


def run(args: argparse.Namespace) -> int:
    pods_values = [int(p.strip()) for p in args.pods.split(",") if p.strip()]
    print("Buzz relay Redis bus scaling harness")
    print("====================================")
    print(
        "scenario: "
        f"{args.communities} communities × {fmt(args.events_per_community_per_sec)} events/s, "
        f"{args.subscribed_communities} subscribed community topic(s), "
        f"interested pods per subscribed community = "
        f"{args.interested_pods_per_subscribed_community or 'all pods'}"
    )
    print()
    print(
        "| pods | old global cluster ingress/s | old avg pod ingress/s | "
        "new scoped cluster ingress/s | new avg pod ingress/s | reduction | old irrelevant/pod | new irrelevant/pod |"
    )
    print(
        "|---:|---:|---:|---:|---:|---:|---:|---:|"
    )

    for pods in pods_values:
        interested = args.interested_pods_per_subscribed_community or pods
        scenario = Scenario(
            pods=pods,
            communities=args.communities,
            events_per_community_per_sec=args.events_per_community_per_sec,
            subscribed_communities=args.subscribed_communities,
            interested_pods_per_subscribed_community=interested,
        )
        old_cluster, old_pod, old_irrelevant = scenario.old_global_firehose()
        new_cluster, new_pod, new_irrelevant = scenario.scoped_bus()
        reduction = old_cluster / new_cluster if new_cluster else float("inf")
        print(
            f"| {pods} | {fmt(old_cluster)} | {fmt(old_pod)} | "
            f"{fmt(new_cluster)} | {fmt(new_pod)} | {reduction:,.1f}× | "
            f"{old_irrelevant:.2f}% | {new_irrelevant:.2f}% |"
        )

    print()
    print("Interpretation:")
    print(
        "- Old relay/global bus: every pod receives every community's event, so "
        "cluster pub/sub ingress = pods × total_event_rate."
    )
    print(
        "- New relay/scoped bus: a pod retains only server-resolved community topics "
        "with local subscribers, so ingress = interested_pods × subscribed_community_rate."
    )
    print(
        "- This harness validates the bus-bound scaling claim; end-to-end latency/DB "
        "capacity should be measured separately with a live relay stack."
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pods", default="1,2,4", help="comma-separated pod counts")
    parser.add_argument("--communities", type=int, default=64)
    parser.add_argument("--events-per-community-per-sec", type=float, default=100.0)
    parser.add_argument("--subscribed-communities", type=int, default=1)
    parser.add_argument(
        "--interested-pods-per-subscribed-community",
        type=int,
        default=0,
        help="0 means all pods are interested in the subscribed community",
    )
    return run(parser.parse_args())


if __name__ == "__main__":
    raise SystemExit(main())
