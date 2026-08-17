"""Bounded model of the shipped stateful public-gateway authority plane.

The model deliberately excludes the relay matcher (not shipped here) and checks the
linearization rules that the gateway does ship: current epoch/generation authority,
relay confinement, expiry, replay admission, quota charging, terminal burn versus
transient release, revocation ordering, custody, and the constant APNs body.
"""
from itertools import permutations, product

FIXED_BODY = b'{"aps":{"alert":{"body":"Reconnect to your relay now"},"mutable-content":1}}'

class Gateway:
    def __init__(self):
        self.relay = "relay-a"
        self.epoch = 1
        self.generation = 1
        self.revoked = False
        self.installation_expires = 100
        self.grant_expires = 80
        self.auth_replays = set()
        self.request_replays = set()
        self.quota = 0
        self.sends = []

    def admit(self, relay="relay-a", epoch=1, generation=1, now=10,
              request_expires=50, auth_id="auth-1", request_id="request-1",
              custody_ok=True):
        if (self.revoked or relay != self.relay or epoch != self.epoch or
            generation != self.generation or now > self.installation_expires or
            now > self.grant_expires or now > request_expires or
            request_expires > self.grant_expires or auth_id in self.auth_replays or
            request_id in self.request_replays):
            return False
        # One durable admission commit: both replay fences and non-refundable quota.
        self.auth_replays.add(auth_id)
        self.request_replays.add(request_id)
        self.quota += 1
        if not custody_ok:
            self.finish(request_id, "transient")
            return False
        self.sends.append((request_id, FIXED_BODY))
        return True

    def finish(self, request_id, outcome):
        if outcome == "transient":
            self.request_replays.discard(request_id)
        elif outcome != "terminal":
            raise ValueError(outcome)

    def rotate(self):
        self.epoch += 1

    def revoke(self):
        self.revoked = True


def explore():
    checked = 0
    for relay_ok, epoch_ok, gen_ok, grant_live, request_live, custody_ok in product(
        [False, True], repeat=6
    ):
        checked += 1
        g = Gateway()
        admitted = g.admit(
            relay="relay-a" if relay_ok else "relay-b",
            epoch=1 if epoch_ok else 0,
            generation=1 if gen_ok else 0,
            now=10,
            request_expires=50 if request_live else 9,
            custody_ok=custody_ok,
        ) if grant_live else g.admit(now=81)
        expected = all((relay_ok, epoch_ok, gen_ok, grant_live, request_live, custody_ok))
        assert admitted == expected
        assert all(body == FIXED_BODY for _, body in g.sends)  # fixed-body noninterference

    # Whichever authority mutation commits first determines admission.
    for actions in permutations(("admit", "revoke")):
        checked += 1
        g = Gateway(); result = None
        for action in actions:
            result = g.admit() if action == "admit" else (g.revoke() or result)
        assert result == (actions[0] == "admit")

    # Terminal outcomes burn the request; transient outcomes release only request-id,
    # while every auth event remains burned and every admitted attempt charges quota.
    for outcome in ("terminal", "transient"):
        checked += 1
        g = Gateway(); assert g.admit()
        g.finish("request-1", outcome)
        assert not g.admit(auth_id="auth-1", request_id="request-2")
        retry = g.admit(auth_id="auth-2", request_id="request-1")
        assert retry == (outcome == "transient")
        assert g.quota == (2 if retry else 1)

    # Rotation invalidates old grants; a current grant remains relay-confined.
    g = Gateway(); g.rotate(); checked += 1
    assert not g.admit(epoch=1)
    assert g.admit(epoch=2)
    return checked

if __name__ == "__main__":
    n = explore()
    print(f"stateful delivery combinations/interleavings checked: {n}")
    print("stateful gateway invariants: HOLD")
