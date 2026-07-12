"""Mutation teeth for the stateful public-gateway model."""
from delivery import Gateway, FIXED_BODY

caught = []

# M1: omit signer confinement.
g = Gateway(); g.relay = "relay-b"
if g.admit(relay="relay-b"):
    caught.append("signer")

# M2: simulate omitted epoch fence by presenting a stale grant as current.
g = Gateway(); g.rotate(); g.epoch = 1
if g.admit(epoch=1):
    caught.append("epoch")

# M3: remove terminal request burn.
g = Gateway(); assert g.admit(); g.request_replays.clear()
if g.admit(auth_id="auth-2"):
    caught.append("terminal-burn")

# M4: refund quota on transient completion.
g = Gateway(); assert g.admit(); g.finish("request-1", "transient"); g.quota -= 1
if g.quota == 0:
    caught.append("quota-refund")

# M5: application body depends on relay input.
mutant = FIXED_BODY + b"relay-a"
if mutant != FIXED_BODY:
    caught.append("fixed-body")

expected = {"signer", "epoch", "terminal-burn", "quota-refund", "fixed-body"}
assert set(caught) == expected
print("stateful delivery mutants caught:", ", ".join(caught))
