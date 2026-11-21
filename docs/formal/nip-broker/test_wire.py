"""Run: python3 -m unittest discover -s docs/formal/nip-broker -v"""
import copy
import json
from pathlib import Path
import unittest
from unittest.mock import patch

import wire

PUB = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
CHANNEL = "5df7dfa8-e919-43df-8efd-f1dcb8af7071"
EVENT = "a" * 64


def envelope(action, args):
    return dict(type="broker_request", protocolVersion=1, requestId="example-1",
                actionVersion=1, action=action, args=args)


def success(req, outcome):
    return dict(type="broker_result", protocolVersion=1, requestId=req["requestId"],
                status="succeeded", action=req["action"], outcome=outcome)


class WireTests(unittest.TestCase):
    def setUp(self):
        self.examples = json.loads(Path(__file__).with_name("vectors.json").read_text())

    def test_all_fifteen_actions(self):
        self.assertEqual(set(wire.SCHEMAS), {v["request"]["action"] for v in self.examples})
        for v in self.examples:
            with self.subTest(action=v["request"]["action"]):
                req = wire.request(wire.compact(v["request"]))
                wire.result(wire.compact(v["result"]), req)

    def test_unknown_null_and_duplicate_at_every_object(self):
        def objects(value, path=()):
            if isinstance(value, dict):
                yield path
                for key, child in value.items():
                    yield from objects(child, path + (key,))
            elif isinstance(value, list):
                for i, child in enumerate(value):
                    yield from objects(child, path + (i,))
        for v in self.examples:
            for side in ("request", "result"):
                def check(raw):
                    return wire.request(raw) if side == "request" else wire.result(raw, v["request"])
                for path in objects(v[side]):
                    for field, value in (("unknown", True), ("unknown", None)):
                        changed = copy.deepcopy(v[side])
                        target = changed
                        for key in path:
                            target = target[key]
                        target[field] = value
                        with self.assertRaises(ValueError):
                            check(wire.compact(changed))
                    target = v[side]
                    for key in path:
                        target = target[key]
                    if target:
                        first = next(iter(target))
                        encoded = wire.compact(target)
                        duplicate = b"{" + wire.compact(first) + b":" + wire.compact(target[first]) + b"," + encoded[1:]
                        raw = wire.compact(v[side]).replace(encoded, duplicate, 1)
                        with self.assertRaises(ValueError):
                            check(raw)
                        for field in target:
                            changed = copy.deepcopy(v[side])
                            item = changed
                            for key in path:
                                item = item[key]
                            item[field] = None
                            with self.assertRaises(ValueError):
                                check(wire.compact(changed))

    def test_error_matrix(self):
        req = self.examples[0]["request"]
        for code in wire.CODES:
            for status in ("failed", "indeterminate"):
                r = dict(type="broker_result", protocolVersion=1, requestId=req["requestId"],
                         status=status, error=dict(code=code, message="no details"))
                valid = code == "internal" or ((code == "outcome_unknown") == (status == "indeterminate"))
                if valid:
                    wire.result(wire.compact(r), req)
                else:
                    with self.assertRaises(ValueError):
                        wire.result(wire.compact(r), req)

    def test_correlation(self):
        for v in self.examples:
            for field, value in (("requestId", "other"), ("action", "unknown"), ("protocolVersion", 2)):
                changed = copy.deepcopy(v["result"])
                changed[field] = value
                with self.assertRaises(ValueError):
                    wire.result(wire.compact(changed), v["request"])

    def test_boundaries(self):
        def valid(action, args):
            return wire.request(wire.compact(envelope(action, args)))
        for n in (1, 500):
            valid("channel.read", dict(channelId=CHANNEL, limit=n))
        for n in (0, 501, True, 1.0):
            with self.assertRaises(ValueError):
                valid("channel.read", dict(channelId=CHANNEL, limit=n))
        for content in ("", " \n", "a" * 65537, "é" * 32769):
            with self.assertRaises(ValueError):
                valid("message.post", dict(channelId=CHANNEL, content=content))
        valid("message.post", dict(channelId=CHANNEL, content="é" * 32768))
        with self.assertRaises(ValueError):
            valid("message.post", dict(channelId=CHANNEL, content="x", mentions=[]))
        with self.assertRaises(ValueError):
            valid("message.post", dict(channelId=CHANNEL, content="x", mentions=["f" * 64]))
        for action, args in (("profile.set", {}), ("agents.update", {"target": {"pubkey": PUB}}),
                             ("observer.emit", {"frames": []})):
            with self.assertRaises(ValueError):
                valid(action, args)
        # Complete NIP-AE body, not just raw value length.
        overhead = len(wire.compact({"slug": "core", "profile": ""}))
        valid("storage.put", {"slug": "core", "value": "x" * (65535 - overhead)})
        with self.assertRaises(ValueError):
            valid("storage.put", {"slug": "core", "value": "x" * (65536 - overhead)})
        with self.assertRaises(ValueError):
            valid("storage.put", {"slug": "core", "value": "\n" * 33000 + "x"})
        req = valid("observer.emit", {"frames": [{"kind": "x", "payload": "{}"}]})
        with self.assertRaises(ValueError):
            wire.result(wire.compact(success(req, {"accepted": 2})), req)

    def test_absence_and_opaque_content(self):
        req = envelope("storage.get", {"slug": "core"})
        for outcome in ({}, {"value": ""}, {"value": '{"anything":null}'}):
            wire.result(wire.compact(success(req, outcome)), req)
        wire.request(wire.compact(envelope("observer.emit", {"frames": [{"kind": "x", "payload": '{"owner":null}'}]})))

    def test_retry_is_bytes_not_equivalent_json(self):
        import hashlib
        request = self.examples[0]["request"]
        frozen = wire.compact(request)
        other = json.dumps(request, indent=2).encode()
        self.assertEqual(wire.request(frozen), wire.request(other))
        self.assertNotEqual(hashlib.sha256(frozen).digest(), hashlib.sha256(other).digest())

    def test_encoding_and_canonical_output(self):
        raw = wire.compact(self.examples[0]["request"])
        with self.assertRaises(ValueError):
            wire.request(raw.decode().encode("utf-16"))
        for action, args in (("storage.get", {"slug": " core "}),
                             ("presence.set", {"status": " online "}),
                             ("agents.delete", {"target": {"pubkey": " " + PUB.upper() + " "}})):
            wire.request(wire.compact(envelope(action, args)))
        for invalid in (CHANNEL.replace("-", "", 1), "{{" + CHANNEL + "}}"):
            with self.assertRaises(ValueError):
                wire.request(wire.compact(envelope("typing.set", {"channelId": invalid})))
        req = envelope("message.post", {"channelId": CHANNEL, "content": "x"})
        for event_id in (EVENT.upper(), " " + EVENT + " "):
            with self.assertRaises(ValueError):
                wire.result(wire.compact(success(req, {"eventId": event_id, "kind": 9, "createdAt": 1})), req)

    def test_mutation_guard_is_load_bearing(self):
        # Deliberately bypass closed-object validation in the oracle, then show
        # the exact negative vector is accepted. This is not a production test.
        changed = copy.deepcopy(self.examples[0]["request"])
        changed["scope"] = "invented-authority"
        raw = wire.compact(changed)
        with self.assertRaises(ValueError):
            wire.request(raw)
        with patch.object(wire, "obj", lambda *args: None):
            self.assertEqual(wire.request(raw)["scope"], "invented-authority")


if __name__ == "__main__":
    unittest.main()
