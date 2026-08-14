#!/usr/bin/env python3
"""Reproducible Buzz relay Redis bus scaling harness.

The harness isolates the relay's Redis fan-out boundary, not client rendering or
DB ingest. It compares the pre-rewrite global-firehose shape (every pod receives
every published event) with the multi-tenant scoped-topic shape (a pod receives
only community topics it has retained because it has local subscribers).

Default scenario:
  * 64 communities publish an equal one-second-equivalent event count.
  * one target community has local subscribers on every relay pod.
  * all other communities are irrelevant to those pods.

Modes:
  * redis: measured Redis PUB/SUB delivery using only Python stdlib (default).
  * model: deterministic no-service arithmetic model.
"""

from __future__ import annotations

import argparse
import os
import socket
import threading
import time
from dataclasses import dataclass
from typing import BinaryIO
from urllib.parse import urlparse
from uuid import UUID

BUZZ_PREFIX = "buzz"
OLD_GLOBAL_CHANNEL = f"{BUZZ_PREFIX}:global"


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
        return cluster, per_pod, 0.0


@dataclass(frozen=True)
class Measurement:
    pods: int
    old_cluster: int
    old_avg_pod: float
    old_irrelevant_pct: float
    new_cluster: int
    new_avg_pod: float
    new_irrelevant_pct: float
    reduction: float
    published: int


@dataclass(frozen=True)
class RedisAddress:
    host: str
    port: int
    db: int
    username: str | None = None
    password: str | None = None


def fmt(value: float) -> str:
    if abs(value - round(value)) < 1e-9:
        return f"{int(round(value)):,}"
    return f"{value:,.2f}"


def community_id(index: int) -> str:
    """Deterministic UUIDs matching the relay's topic format."""
    return str(UUID(int=index + 1))


def scoped_global_channel(community: str) -> str:
    # Mirrors crates/buzz-pubsub/src/topic.rs EventTopic::Global:
    # format!("buzz:{}:global", self.community_id)
    return f"{BUZZ_PREFIX}:{community}:global"


def encode_command(*parts: object) -> bytes:
    encoded = [str(part).encode("utf-8") for part in parts]
    frame = [f"*{len(encoded)}\r\n".encode("ascii")]
    for part in encoded:
        frame.append(f"${len(part)}\r\n".encode("ascii"))
        frame.append(part)
        frame.append(b"\r\n")
    return b"".join(frame)


def _read_line(stream: BinaryIO) -> bytes:
    line = stream.readline()
    if not line:
        raise EOFError("Redis connection closed")
    if not line.endswith(b"\r\n"):
        raise ValueError(f"malformed Redis line: {line!r}")
    return line[:-2]


def read_resp(stream: BinaryIO) -> object:
    prefix = stream.read(1)
    if not prefix:
        raise EOFError("Redis connection closed")
    if prefix == b"+":
        return _read_line(stream).decode("utf-8")
    if prefix == b"-":
        raise RuntimeError(_read_line(stream).decode("utf-8"))
    if prefix == b":":
        return int(_read_line(stream))
    if prefix == b"$":
        length = int(_read_line(stream))
        if length == -1:
            return None
        data = stream.read(length)
        terminator = stream.read(2)
        if len(data) != length or terminator != b"\r\n":
            raise ValueError("malformed Redis bulk string")
        return data.decode("utf-8")
    if prefix == b"*":
        length = int(_read_line(stream))
        if length == -1:
            return None
        return [read_resp(stream) for _ in range(length)]
    raise ValueError(f"unknown Redis RESP prefix: {prefix!r}")


class RedisClient:
    def __init__(self, address: RedisAddress, timeout: float) -> None:
        self._address = address
        self._sock = socket.create_connection((address.host, address.port), timeout=timeout)
        self._sock.settimeout(timeout)
        self._stream = self._sock.makefile("rwb")
        self._authenticate_and_select()

    def _authenticate_and_select(self) -> None:
        if self._address.password:
            if self._address.username:
                self.command("AUTH", self._address.username, self._address.password)
            else:
                self.command("AUTH", self._address.password)
        if self._address.db:
            self.command("SELECT", self._address.db)

    def command(self, *parts: object) -> object:
        self._stream.write(encode_command(*parts))
        self._stream.flush()
        return read_resp(self._stream)

    def close(self) -> None:
        try:
            self._stream.close()
        finally:
            self._sock.close()

    def __enter__(self) -> "RedisClient":
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()


class RedisSubscriber(threading.Thread):
    def __init__(
        self,
        *,
        pod: int,
        address: RedisAddress,
        channels: list[str],
        relevant_communities: set[str],
        timeout: float,
    ) -> None:
        super().__init__(name=f"redis-sub-pod-{pod}", daemon=True)
        self.address = address
        self.channels = channels
        self.relevant_communities = relevant_communities
        self.timeout = timeout
        self.ready = threading.Event()
        self.stop_requested = threading.Event()
        self.error: BaseException | None = None
        self.received = 0
        self.irrelevant = 0
        self._sock: socket.socket | None = None

    def run(self) -> None:
        try:
            sock = socket.create_connection((self.address.host, self.address.port), timeout=self.timeout)
            self._sock = sock
            sock.settimeout(0.2)
            stream = sock.makefile("rwb")
            try:
                if self.address.password:
                    if self.address.username:
                        self._write_and_read(stream, "AUTH", self.address.username, self.address.password)
                    else:
                        self._write_and_read(stream, "AUTH", self.address.password)
                if self.address.db:
                    self._write_and_read(stream, "SELECT", self.address.db)

                stream.write(encode_command("SUBSCRIBE", *self.channels))
                stream.flush()
                subscribed = 0
                while subscribed < len(self.channels):
                    frame = read_resp(stream)
                    if isinstance(frame, list) and frame and frame[0] == "subscribe":
                        subscribed += 1
                self.ready.set()

                while not self.stop_requested.is_set():
                    try:
                        frame = read_resp(stream)
                    except socket.timeout:
                        continue
                    if not (isinstance(frame, list) and len(frame) >= 3 and frame[0] == "message"):
                        continue
                    payload = str(frame[2])
                    community = payload.split("|", 1)[0]
                    self.received += 1
                    if community not in self.relevant_communities:
                        self.irrelevant += 1
            finally:
                stream.close()
                sock.close()
        except OSError as exc:
            if not self.stop_requested.is_set():
                self.error = exc
                self.ready.set()
        except BaseException as exc:
            self.error = exc
            self.ready.set()

    @staticmethod
    def _write_and_read(stream: BinaryIO, *parts: object) -> object:
        stream.write(encode_command(*parts))
        stream.flush()
        return read_resp(stream)

    def stop(self) -> None:
        self.stop_requested.set()
        if self._sock is not None:
            try:
                self._sock.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            try:
                self._sock.close()
            except OSError:
                pass


def parse_redis_url(raw: str) -> RedisAddress:
    parsed = urlparse(raw)
    if parsed.scheme not in {"redis", "rediss"}:
        raise ValueError(f"unsupported Redis URL scheme: {parsed.scheme!r}")
    if parsed.scheme == "rediss":
        raise ValueError("rediss:// is not supported by this stdlib harness; use redis://")
    return RedisAddress(
        host=parsed.hostname or "127.0.0.1",
        port=parsed.port or 6379,
        db=int(parsed.path.lstrip("/") or "0"),
        username=parsed.username,
        password=parsed.password,
    )


def publish_scenario(
    *,
    address: RedisAddress,
    channels_by_community: list[str],
    events_per_community: int,
    timeout: float,
) -> None:
    with RedisClient(address, timeout) as redis:
        for community_index, channel in enumerate(channels_by_community):
            community = community_id(community_index)
            for event_index in range(events_per_community):
                redis.command("PUBLISH", channel, f"{community}|{event_index}")


def wait_until_ready(subscribers: list[RedisSubscriber], timeout: float) -> None:
    for sub in subscribers:
        if not sub.ready.wait(timeout):
            stop_subscribers(subscribers)
            raise TimeoutError(f"subscriber {sub.name} did not SUBSCRIBE within {timeout}s")
        if sub.error is not None:
            stop_subscribers(subscribers)
            raise RuntimeError(f"subscriber {sub.name} failed: {sub.error}")


def wait_for_counts(subscribers: list[RedisSubscriber], expected_total: int, timeout: float) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if any(sub.error for sub in subscribers):
            break
        if sum(sub.received for sub in subscribers) >= expected_total:
            return
        time.sleep(0.01)
    actual = sum(sub.received for sub in subscribers)
    raise TimeoutError(f"timed out waiting for {expected_total} pub/sub deliveries; got {actual}")


def stop_subscribers(subscribers: list[RedisSubscriber]) -> None:
    for sub in subscribers:
        sub.stop()
    for sub in subscribers:
        sub.join(timeout=1.0)


def measure_redis_for_pods(args: argparse.Namespace, pods: int) -> Measurement:
    events_per_community = int(args.events_per_community_per_sec)
    if events_per_community != args.events_per_community_per_sec:
        raise ValueError("--mode redis requires integer --events-per-community-per-sec")
    address = parse_redis_url(args.redis_url)
    communities = [community_id(i) for i in range(args.communities)]
    relevant = set(communities[: args.subscribed_communities])
    interested = pods if args.interested_pods_per_subscribed_community == 0 else min(
        pods, args.interested_pods_per_subscribed_community
    )

    with RedisClient(address, args.redis_timeout) as redis:
        assert redis.command("PING") == "PONG"

    old_subs = [
        RedisSubscriber(
            pod=pod,
            address=address,
            channels=[OLD_GLOBAL_CHANNEL],
            relevant_communities=relevant,
            timeout=args.redis_timeout,
        )
        for pod in range(pods)
    ]
    for sub in old_subs:
        sub.start()
    wait_until_ready(old_subs, args.redis_timeout)
    publish_scenario(
        address=address,
        channels_by_community=[OLD_GLOBAL_CHANNEL for _ in communities],
        events_per_community=events_per_community,
        timeout=args.redis_timeout,
    )
    wait_for_counts(old_subs, pods * args.communities * events_per_community, args.redis_timeout)
    stop_subscribers(old_subs)

    new_subs = [
        RedisSubscriber(
            pod=pod,
            address=address,
            channels=[scoped_global_channel(community) for community in communities[: args.subscribed_communities]],
            relevant_communities=relevant,
            timeout=args.redis_timeout,
        )
        for pod in range(interested)
    ]
    for sub in new_subs:
        sub.start()
    wait_until_ready(new_subs, args.redis_timeout)
    publish_scenario(
        address=address,
        channels_by_community=[scoped_global_channel(community) for community in communities],
        events_per_community=events_per_community,
        timeout=args.redis_timeout,
    )
    wait_for_counts(new_subs, interested * args.subscribed_communities * events_per_community, args.redis_timeout)
    stop_subscribers(new_subs)

    old_cluster = sum(sub.received for sub in old_subs)
    new_cluster = sum(sub.received for sub in new_subs)
    old_irrelevant = sum(sub.irrelevant for sub in old_subs)
    new_irrelevant = sum(sub.irrelevant for sub in new_subs)
    published = args.communities * events_per_community
    return Measurement(
        pods=pods,
        old_cluster=old_cluster,
        old_avg_pod=old_cluster / pods,
        old_irrelevant_pct=100.0 * old_irrelevant / old_cluster if old_cluster else 0.0,
        new_cluster=new_cluster,
        new_avg_pod=new_cluster / pods,
        new_irrelevant_pct=100.0 * new_irrelevant / new_cluster if new_cluster else 0.0,
        reduction=old_cluster / new_cluster if new_cluster else float("inf"),
        published=published,
    )


def model_measurements(args: argparse.Namespace, pods_values: list[int]) -> list[Measurement]:
    rows = []
    for pods in pods_values:
        scenario = Scenario(
            pods=pods,
            communities=args.communities,
            events_per_community_per_sec=args.events_per_community_per_sec,
            subscribed_communities=args.subscribed_communities,
            interested_pods_per_subscribed_community=args.interested_pods_per_subscribed_community or pods,
        )
        old_cluster, old_pod, old_irrelevant = scenario.old_global_firehose()
        new_cluster, new_pod, new_irrelevant = scenario.scoped_bus()
        rows.append(
            Measurement(
                pods=pods,
                old_cluster=int(old_cluster),
                old_avg_pod=old_pod,
                old_irrelevant_pct=old_irrelevant,
                new_cluster=int(new_cluster),
                new_avg_pod=new_pod,
                new_irrelevant_pct=new_irrelevant,
                reduction=old_cluster / new_cluster if new_cluster else float("inf"),
                published=int(scenario.total_event_rate),
            )
        )
    return rows


def redis_measurements(args: argparse.Namespace, pods_values: list[int]) -> list[Measurement]:
    return [measure_redis_for_pods(args, pods) for pods in pods_values]


def assert_scaling(args: argparse.Namespace, rows: list[Measurement]) -> None:
    ideal = args.communities / args.subscribed_communities
    min_reduction = ideal * args.min_reduction_ratio
    for row in rows:
        if row.reduction < min_reduction:
            raise AssertionError(
                f"pods={row.pods}: reduction {row.reduction:.2f}× < required {min_reduction:.2f}×"
            )
        if row.new_irrelevant_pct > args.max_scoped_irrelevant_pct:
            raise AssertionError(
                f"pods={row.pods}: scoped irrelevant {row.new_irrelevant_pct:.2f}% > "
                f"allowed {args.max_scoped_irrelevant_pct:.2f}%"
            )


def print_rows(args: argparse.Namespace, rows: list[Measurement]) -> None:
    print("Buzz relay Redis bus scaling harness")
    print("====================================")
    print(
        "scenario: "
        f"{args.communities} communities × {fmt(args.events_per_community_per_sec)} events/s, "
        f"{args.subscribed_communities} subscribed community topic(s), "
        f"interested pods per subscribed community = "
        f"{args.interested_pods_per_subscribed_community or 'all pods'}, "
        f"mode={args.mode}"
    )
    if args.mode == "redis":
        print(f"redis: {args.redis_url}")
    print()
    print(
        "| pods | old global cluster ingress/s | old avg pod ingress/s | "
        "new scoped cluster ingress/s | new avg pod ingress/s | reduction | old irrelevant/pod | new irrelevant/pod |"
    )
    print("|---:|---:|---:|---:|---:|---:|---:|---:|")
    for row in rows:
        print(
            f"| {row.pods} | {fmt(row.old_cluster)} | {fmt(row.old_avg_pod)} | "
            f"{fmt(row.new_cluster)} | {fmt(row.new_avg_pod)} | {row.reduction:,.1f}× | "
            f"{row.old_irrelevant_pct:.2f}% | {row.new_irrelevant_pct:.2f}% |"
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
        "- The assertion checks the bus-bound scaling claim; end-to-end latency/DB "
        "capacity should be measured separately with a live relay stack."
    )


def run(args: argparse.Namespace) -> int:
    if args.communities <= 0:
        raise ValueError("--communities must be positive")
    if args.subscribed_communities <= 0:
        raise ValueError("--subscribed-communities must be positive")
    if args.subscribed_communities > args.communities:
        raise ValueError("--subscribed-communities cannot exceed --communities")
    pods_values = [int(p.strip()) for p in args.pods.split(",") if p.strip()]
    if not pods_values or any(p <= 0 for p in pods_values):
        raise ValueError("--pods must contain positive integers")

    rows = model_measurements(args, pods_values) if args.mode == "model" else redis_measurements(args, pods_values)
    print_rows(args, rows)
    if args.assert_scaling:
        assert_scaling(args, rows)
        print()
        print(
            "assertion: PASS "
            f"(reduction ≥ {args.min_reduction_ratio:.0%} of ideal, "
            f"scoped irrelevant ≤ {args.max_scoped_irrelevant_pct:.2f}%)"
        )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=["redis", "model"], default="redis")
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
    parser.add_argument("--redis-url", default=os.environ.get("REDIS_URL", "redis://127.0.0.1:6379/0"))
    parser.add_argument("--redis-timeout", type=float, default=10.0)
    parser.add_argument(
        "--no-assert-scaling",
        dest="assert_scaling",
        action="store_false",
        help="print measurements without enforcing the scaling invariant",
    )
    parser.set_defaults(assert_scaling=True)
    parser.add_argument(
        "--min-reduction-ratio",
        type=float,
        default=0.95,
        help="minimum observed reduction as a fraction of ideal communities/subscribed",
    )
    parser.add_argument("--max-scoped-irrelevant-pct", type=float, default=0.0)
    return run(parser.parse_args())


if __name__ == "__main__":
    raise SystemExit(main())
