---
title: "NIP-RS manual-unread: bounded exhaustive model — candidates A vs B"
tags: [nostr, nip-rs, read-state, formal-model, buzz]
status: active
created: 2026-07-16
---

# NIP-RS manual-unread encoding model

Bounded exhaustive model comparing two candidate CRDT encodings for a
manual mark-as-unread override layer within NIP-RS read state.

## Run

```bash
python3 exhaustive.py
python3 mutation.py
```

Both scripts are deterministic and exit 0 on success.

## Context

NIP-RS v1 encodes read state as grow-only `max(timestamp)` frontiers per
context. Manual mark-as-unread requires a second source of truth (an
override layer) because the frontier cannot be lowered — a lower value is
indistinguishable from a stale replica under `max()` merge.

The override layer must converge across devices, survive legacy client
rewrite cycles, and remain bounded within the existing 32 KiB plaintext
budget. Two candidate encodings are modeled:

- **A — lexicographic operation register:** per context, one register
  `{counter, client_tiebreak, op, baseline}` in a NEW top-level field.
- **B — two grow-only counters + baseline:** per context, `S` (set
  counter), `C` (clear counter), `B` (frontier-at-set-time) encoded as
  sibling keys under `contexts`.

## Model universe

- 2 upgraded devices + 1 legacy device
- 2 contexts (`c0`, `c1`)
- Actions: mark-unread, mark-read (with frontier advance),
  advance-frontier, compact, reinstall (client_id loss),
  deliver (including duplicate/replay)
- BFS over canonical global states with interleaved actions and deliveries
  (not phased), depth-bounded
- All delivery permutations of published blobs at terminal states
- Multi-slot union (split blob across 2 slots, deliver separately)
- Directed deep-history check: compact → new local actions (counter
  reuse) → delayed stale delivery (including split across 2 slots),
  over a 2,016-point parameter cube (stale `(S,C,B)` × post-compaction
  frontier × 7 action sequences × 2 tie policies × 3 delivery shapes)
- Cross-device compaction transparency check: same tombstone, delivered
  to an unrelated device with its own live concurrent state, over a
  312-point parameter cube (stale `(S,C,B)` × post-compaction frontier ×
  4 fresh-frontier values × 2 tie policies), plus a monotonicity lemma
  over 1,728 points (2 tie policies × 4×4×3×3 receiving-register/frontier
  combinations × 6 ceiling values) proving the ceiling can never
  *strengthen* a receiving register's set-counter standing
- States explored: 7,129 per tie policy (14,258 total)
- Published-state merge closure: every override is canonicalized against
  the device's own effective frontier at serialization time before
  hitting the wire (mandatory, not optional) — live unchanged, dead
  folded to the tombstone floor, virgin omitted. Checked over a directed
  witness (Thufir's exact dead+dead pair) plus a general search: every
  pairwise join of a bounded cube of independently-dead published states
  (300 points per tie policy), including a one-hop relay republication
  to cover delayed/multi-hop delivery — 45,074 pairs checked total across
  both tie policies

## Invariants checked

| # | Invariant | A | B (clear-wins) | B (set-wins) |
|---|-----------|---|-----------------|--------------|
| I1 | Join associative/commutative/idempotent | PASS | PASS | PASS |
| I2 | Convergence (all delivery orders) | PASS | PASS | PASS |
| I3 | No frontier regression | PASS | PASS | PASS |
| I4 | Concurrent set/clear winner stable | PASS | PASS | PASS |
| I5 | Compaction: no loss, no resurrection (immediate merge-back) | n/a | PASS | PASS |
| I5c | Deep-history: compact → reuse → delayed stale delivery (same-device replay) | n/a | PASS | PASS |
| I5d | Cross-device compaction transparency (suppress-only, not zero-divergence) | n/a | PASS | PASS |
| I5e | Published-state merge closure: dead+dead join stays inactive | n/a | PASS | PASS |
| I6 | Replay harmless | PASS | PASS | PASS |
| I7 | Legacy rewrite safety | **FAIL** (witness) | PASS | PASS |
| I8 | Bounded key growth (3 keys/ctx live, 1 key/ctx tombstone) | n/a | PASS | PASS |
| I9 | DeviceA counter absorption | PASS | n/a | n/a |

I5 covers the immediate compacted-vs-pre-compaction merge shape (both
merge orders). I5c is the same-device deep-history property this round
was originally opened to close: it directly targets the ~9-transition
history a depth-4 BFS cannot structurally reach (compact → new local
set/clear → delayed stale delivery, including from a second slot),
asserting that compaction never resurrects a dead override or drops a
live one **when the delayed delivery is the compacting device's own
pre-compaction ancestor** (or an exact copy of it, e.g. a peer that
never advanced past the original snapshot).

**I5c does not cover, and NOTE.md previously overstated, the
cross-device case.** Compaction is a storage optimization from the
compacting device's own point of view — its dead register's baseline
`B` was frontier-relative to *that device's* history, and dropping `S`
in favor of the `C` ceiling is safe against replays of *its own* past.
But once published, the tombstone's `C` ceiling is globally comparable
via componentwise `max()`, while the baseline-relative death that
produced it is not. I5d proves the resulting property precisely:
merging in a tombstone can **suppress** — never resurrect, per the
`test_tombstone_merge_monotonic` structural lemma — a different
device's concurrent fresh set whose own counters happen to be at or
below the tombstone's ceiling, and the suppression always recovers with
one more local mark-unread (verified replay-stable against the same
tombstone). This is a one-shot false-negative risk, not a correctness
violation of the CRDT join (idempotent/commutative/associative still
hold per I1) and not new: an *uncompacted* stale explicit clear already
suppresses a fresh concurrent set under clear-wins with no compaction
anywhere (verified directly — see "Tie policy evidence" below); the
tombstone extends the same false-negative-preferring shape to
baseline-dominated dead sets that were never explicitly cleared.

**I5e — published-state merge closure — is a protocol requirement, not
an optimization.** I5d's suppress-only guarantee assumes the tombstone
was actually on the wire before the merge. Nothing forces that:
`compact_b()`/`do_compact` are a local storage-GC transition a device
may or may not have called before it serializes. Without a mandatory
canonicalization step, `publish_blob()` can emit a register's *raw*
`(S, C, B)` — dead by construction (baseline-dominated, clear-dominated,
or a clear-wins tie) but not yet folded into the tombstone's
globally-comparable `C` ceiling. Two such raw-dead registers, published
by two different devices for unrelated reasons, can componentwise-max
into a **live** join: each register's `S` and `B` came from a different
device history, and the merge recombines them independent of either
history's own death cause. This is a distinct hazard from I5d's
suppression (I5d is a live register losing to a stale dead one; I5e's
witness is two dead registers producing a live one) but the same root
cause — components taken from independent histories can be
recombined in ways neither history's own frontier ever permitted.

**Fix: canonical publication is mandatory, not advisory.**
`DeviceB.publish_blob()` now canonicalizes every override against the
device's own effective frontier at serialization time, unconditionally
— live unchanged (3 keys), dead folded to the tombstone floor `RegB(0,
max(S,C), 0)` (1 key), virgin omitted (0 keys) — regardless of whether
`do_compact` was ever called locally first. This is a **spec-amendment
requirement for any production client implementing this override
layer**: publication MUST canonicalize before serialization, the same
way it MUST advance the frontier monotonically. It is load-bearing
correctness, not a storage optimization a client can opt out of.
`do_compact` remains available separately to mutate a device's own
`self.overrides` for local storage-GC purposes; it is no longer a
prerequisite for correct publication, because publication no longer
depends on prior local state having been compacted.

**Proof obligation closed:** `exhaustive.py::test_published_merge_closure`
checks two ways — Thufir's exact witness pair
(`RegB(3,2,0)`@baseline-dead-50 join `RegB(1,2,100)`@clear-dead-100,
raw join is live `RegB(3,2,100)`) as a directed case under both tie
policies, and a general search over every pairwise join of a
300-point-per-policy cube of independently-dead published states,
including a one-hop relay republication step to cover delayed/multi-hop
delivery (a relay that receives one operand alone and republishes —
re-canonicalizing — before forwarding). `mutation.py::mutant_m7` reverts
`publish_blob` to the pre-fix raw-serialization behavior and reproduces
Thufir's exact resurrection witness directly, confirming the new
invariant has teeth.



### Convergence

Both candidates converge under all tested delivery permutations.
Candidate B achieves this with componentwise `max()` merge (a standard
state-based CRDT join). Candidate A uses a register with lexicographic
tuple comparison — also convergent, but the register requires a
client-identity tiebreak field.

### Legacy compatibility matrix

| Scenario | A | B |
|----------|---|---|
| Upgraded publishes, legacy reads blob | Legacy drops `overrides` field | Legacy preserves `ov_*` sibling keys |
| Legacy rewrites same slot | **Overrides erased** (expected-witness confirmed) | Sibling keys survive sanitization |
| Upgraded reads legacy-rewritten blob | Override state lost | Override state intact |
| Legacy reads its own frontier | Inert (correct) | Inert (correct) |
| Legacy frontier advance past baseline | Cannot clear override (erased) | Stale set dominated (correct) |

**Candidate A's legacy erasure is the decisive defect.** The desktop and
mobile parsers (`readStateFormat.ts:82-108`, `read_state_format.dart:100-141`)
reconstruct only `{v, client_id, contexts}`. A same-slot legacy rewrite
drops the top-level `overrides` field entirely and republishes without it.
There is no safe migration path: any user with a single legacy device
loses all manual-unread state on the next rewrite cycle.

Candidate B's sibling keys (`ov_s:`, `ov_c:`, `ov_b:`) pass all legacy
validation gates — keys are <= 256 UTF-8 bytes, values are uint32 —
and round-trip through legacy rewrite unmodified.

**Legacy carry-through simplification (documented divergence).** Row
"Legacy preserves `ov_*` sibling keys" is proven two different ways in
this model, and they are not the same claim:

- `legacy_sanitize_blob` — the byte-sanitization function alone (drop
  keys >256 UTF-8 bytes or non-uint32 values) — genuinely preserves
  unknown keys as opaque pass-through, matching production
  `sanitizeContexts`. `test_legacy_rewrite_b` (I7) exercises exactly
  this: an upgraded device's blob is sanitized and received by a
  *second upgraded* device; the sibling keys survive because
  sanitization never touches keys it doesn't recognize.
- `DeviceB(is_legacy=True)` — the explorer's legacy *device* object used
  in the multi-device BFS (`exhaustive.py`) — does **not** carry
  through `ov_*` keys it receives. `receive_merge` parses them into a
  local dict but the store step is gated on `not self.is_legacy`
  (`model.py:268`), so a legacy device's own `publish_blob` only ever
  republishes its own frontier keys, never sibling keys it received
  from an upgraded peer. This is a deliberate model simplification, not
  a claim about production: production's legacy client is a single
  `sanitizeContexts` pass with no in-memory override model to gate on,
  so it forwards unknown keys unchanged; the model's `DeviceB` needed an
  explicit legacy/upgraded split to represent "does not understand or
  act on overrides" for the BFS explorer's mark-unread/mark-read action
  space, and that split was implemented as drop-on-receive rather than
  store-opaque-and-forward.
- **Why this doesn't hide a defect:** every invariant that asserts
  sibling-key survival through a legacy hop (I7) is checked via the
  sanitize function directly, never via a `DeviceB(is_legacy=True)`
  relay round-trip — the two paths are never conflated in a single
  assertion. The BFS explorer's own legacy-device transitions are also
  gated: `enabled_transitions` only enqueues `mark_unread`/`mark_read`/
  `compact` for a device `if not d.is_legacy` (`exhaustive.py:118-124`),
  so a legacy device in the BFS never even attempts to act on overrides;
  `do_mark_unread`/`do_mark_read` (`model.py:210-222`) additionally
  carry an explicit `if self.is_legacy: return` no-op guard as
  defense-in-depth for the same property. `do_compact`
  (`model.py:227-236`) carries no such explicit guard — it is a no-op
  for a legacy device only *transitively*, because `self.overrides`
  is never populated for one (every write path into `self.overrides`
  is already gated on `not self.is_legacy`), so `do_compact` finds
  `self.overrides.get(ctx)` is always `None` and returns immediately.
  Either way, the drop-on-receive simplification never
  changes the BFS's own convergence or compaction verdicts (I2, I3, I5,
  I5c, I5d) — those are computed only over upgraded devices'
  `override_is_set`. The one place a real production legacy client
  *does* matter for override survival — sanitizing an upgraded device's
  own re-published blob — is I7's scope, and I7 uses the accurate
  function.
- **Implication for implementation:** production's `sanitizeContexts`
  pass-through behavior is correct and required; this note exists so a
  future reader of `DeviceB.receive_merge` doesn't mistake the model's
  drop-on-receive simplification for a claim that legacy relaying loses
  override state in production — it doesn't, per the function-level
  proof above.

### Identity dependence

- **A requires client_id** for the tiebreak field. After reinstall
  (new `client_id`), the tiebreak changes. Convergence is preserved only
  because the counter is strictly higher; a same-counter reinstall would
  create an ambiguous merge.
- **B needs no client identity** — componentwise `max()` is
  identity-free. Confirmed: reinstall with new `client_id` preserves
  convergence.

### Bytes per manually-unread context

Sizes computed with realistic context IDs. Envelope cost
(`{"v":1,"client_id":"...","contexts":{}}`) is ~60 bytes and shared
across all contexts — amortized to near zero per context.

| Context type | Context ID example | ID length | Live override keys (3) | Tombstone key (1) |
|--------------|-------------------|-----------|------------------------|--------------------|
| Channel | `b68cd7cb-6f8d-4641-b743-a7349eb4114b` | 36 | 138 bytes | 45 bytes |
| Message | `msg:` + 64-hex event ID | 68 | 234 bytes | 77 bytes |
| Thread | `thread:` + 64-hex event ID | 71 | 243 bytes | 80 bytes |

Live-override bytes are unchanged by the reserved-namespace escaping
(below): every context ID Buzz actually generates (channel UUID,
`msg:hex64`, `thread:hex64`) is a no-op under `escape_context_key` — none
begin with `ov_` or `esc:` — so the escape marker costs 0 bytes in the
common case. Tombstone bytes are new in this revision: canonical
publication no longer serializes a dead register at 3 keys (see
"Compaction behavior" below and "Published-state merge closure" above)
but a single `ov_c:` key with the counter ceiling — this is now the
literal output of `publish_blob()` for any dead override, not merely
the output of the optional `do_compact` storage-GC step.

Breakdown for channel context (worst real-world common case, live):
```
"ov_s:b68cd7cb-6f8d-4641-b743-a7349eb4114b":1    → 44 chars
"ov_c:b68cd7cb-6f8d-4641-b743-a7349eb4114b":0    → 44 chars
"ov_b:b68cd7cb-6f8d-4641-b743-a7349eb4114b":10   → 45 chars
                                          total  ≈ 138 bytes (+ 2 commas)
```

Tombstone floor for channel context (dead override after compaction):
```
"ov_c:b68cd7cb-6f8d-4641-b743-a7349eb4114b":3    → 45 chars ≈ 45 bytes
```

Candidate A for comparison: `{"counter":1,"tiebreak":"dev0","op":"SET","baseline":10}`
≈ 56 bytes per context as a JSON object, plus the top-level `overrides`
field overhead. However, this is moot since A's top-level field is erased
by legacy clients.

### Reserved key namespace

NIP-RS v1 context IDs are arbitrary UTF-8 (spec `:89`, `:113-114`), so a
pre-existing opaque context could legitimately begin with `ov_s:`,
`ov_c:`, or `ov_b:` and, once flattened into the same `contexts` map,
be misparsed as a control key for a *different* context.

**Reservation:** the 3-byte stem `ov_` and the escape marker `esc:` are
reserved at the spec-amendment level. A raw context ID that begins with
either is escaped on publish by prepending `esc:`, and unescaped on
receive by stripping exactly one leading `esc:` (`model.py:
escape_context_key`, `unescape_context_key`). This is a bijection, not
an idempotent no-op: a context literally named `esc:foo` escapes to
`esc:esc:foo` on the wire and unescapes back to exactly `esc:foo` on
receipt — the two operations are inverses, so no collision or data
loss occurs even for context IDs that already contain the marker.

**Cost:** zero bytes for every context ID Buzz generates today (channel
UUID, `msg:hex64`, `thread:hex64` — none start with `ov_` or `esc:`).
Only a context ID that happens to start with the reserved stem pays the
4-byte `esc:` prefix.

**Backward-compatibility limitation (Thufir's qualification — not a
collision-safe migration of existing data):** a context published
*unescaped* by a client that predates this amendment, and that happens
to start with `ov_` (e.g. an already-published, pre-existing
`ov_s:evil`-style context), is **not safely migrated** by this scheme.
Retroactive escaping cannot rewrite a blob the original publisher never
knew needed escaping — the codec protects contexts generated by
amendment-aware clients going forward, not history that predates the
amendment. This is a theoretical concern for the reasons in the
">256-byte key drop hazard" section: Buzz's own key shapes cannot
trigger it, and no legacy client is known to generate `ov_`-prefixed
context IDs. Documented as a residual, unsolved, backward-compatibility
gap — not modeled further — per the same practical-risk reasoning
already applied to the 256-byte hazard below.

**Verified:** `exhaustive.py::test_reserved_namespace_collision` — a
context literally named `ov_s:evil` round-trips through publish/receive
as frontier state (not misparsed as an override), and a real override on
a *different* context in the same blob is unaffected.

### Counter headroom (uint32)

Each counter (S, C) is a uint32: 2^32 - 1 = 4,294,967,295. At one
toggle per second, ~136 years. No practical concern for manual
right-click actions.

### >256-byte key drop hazard

Legacy `sanitizeContexts` drops any key with `len(key.encode('utf-8')) > 256`.
Adding the `ov_s:` prefix (5 bytes) to a context key creates a key of
`len(context_id) + 5` bytes. If the original context key is at or near
the 256-byte limit, the prefixed override key exceeds it and is silently
dropped by legacy sanitization.

In practice, context keys are UUIDs (36 bytes), hex event IDs (64-68 bytes),
or thread IDs (71 bytes) — all well under 256 bytes. The longest common
override key (`ov_b:thread:` + 64-hex = 76 bytes) has 180 bytes of
headroom. This hazard is theoretical but should be documented in the spec.

### 10,000-key validation limit

Legacy `isValidBlob` rejects blobs with >10,000 context keys. Live
override keys consume 3 entries per overridden context; a compacted
(tombstoned) override consumes 1:

| Overridden contexts | Live override keys | Typical frontier keys | Total | Headroom |
|--------------------|---------------------|-----------------------|-------|----------|
| 50 | 150 | ~500 | 650 | 93.5% |
| 100 | 300 | ~1,000 | 1,300 | 87% |
| 500 | 1,500 | ~2,000 | 3,500 | 65% |
| 3,000 | 9,000 | ~1,000 | 10,000 | 0% (limit) |

The 32 KiB byte budget is the binding constraint long before key count.

### Compaction behavior (tombstone-floor, policy-dependent)

**Revision note:** the prior "compacts to zero" design (delete-on-
dominance: a dead register was dropped entirely, 0 keys) is retracted.
Thufir's pass-3 review found a stale-replay resurrection: dropping all
`(S,C)` state made counters reusable, so a new local set/clear pair
restarting from `S=0,C=0` could be dominated by a delayed stale peer
snapshot on replay (`RegB(3,0,10)` → compact → `None` → local
set+clear → `RegB(1,2,20)` → stale replay merges in → `RegB(3,2,20)`,
`S>C`, resurrected). Fixed by a tombstone floor: any register with
recorded activity (S>0 or C>0) is *never* fully deleted — dead state
compacts to `RegB(0, max(S,C), 0)` instead of `None`. Only a virgin
register (never set, S==0 and C==0) has no ceiling to protect and
compacts to `None`.

**The compaction rule is now uniform across the dead cases — the
per-branch table collapses to a single test:**

| Condition | Clear-wins | Set-wins |
|-----------|-----------|----------|
| `override_set_b(reg)` is True (live) | Do not compact | Do not compact |
| `override_set_b(reg)` is False and `S>0 or C>0` (dead, ever-active) | Compact to tombstone floor `RegB(0, max(S,C), 0)` | Compact to tombstone floor (same) |
| `S == 0, C == 0` (virgin, never set) | Drop entirely (`None`) | Drop entirely (same) |

Because `override_set_b` is already policy-aware, "live" vs. "dead"
differs by policy exactly where it did before (`S == C, S > 0` is dead
under clear-wins, live under set-wins) — the tombstone floor rule itself
does not need to branch on policy; `compact_b` calls `override_set_b`
once and only tombstones the false branch.

Under clear-wins, a dead override compacts to the ~45-byte tombstone
(one `ov_c:` key, channel context) — **not** to zero, because `C` must
persist as the reuse-blocking ceiling. Under set-wins, `S == C` overrides
remain live and are never compacted (3 keys, ~138 bytes for channel
contexts) — unchanged from the prior revision.

**Proof obligation closed (same-device replay):**
`exhaustive.py::test_deep_history_compaction` (2,016-point parameter
cube) and `test_tombstone_stale_merge_direct` verify no resurrection
and no loss of a genuinely-live override across the compact →
new-action → delayed-stale-delivery shape, for both single-slot and
2-slot-split delivery, both tie policies. `mutation.py::mutant_m4`
reverts to the old delete-on-dominance rule and reproduces the exact
resurrection witness (`final_reg=RegB(s=3, c=2, b=20)`,
`override_is_set=True`) — confirming the suite would have caught the
defect this round was opened to fix.

**Proof obligation closed (cross-device transparency, requalified —
suppress-only, not zero-divergence):**
`exhaustive.py::test_cross_device_compaction_suppression` (312-point
cube: stale ancestor `(S,C,B)` × post-compaction frontier × 4
fresh-frontier values on the receiving device × 2 tie policies) proves
every divergence between "receive the tombstone" and "receive the
uncompacted ancestor" is a suppression of an unrelated device's live
set — never a resurrection — and that every suppression recovers with
one more local mark-unread and stays recovered after re-receiving the
same tombstone. `test_tombstone_merge_monotonic` proves the direction
structurally (not just over the bounded cube): merging in a tombstone
`RegB(0, k, 0)` for any ceiling `k` can only raise the receiving
register's `C`, never its `S` or `B`, so it can only weaken — never
strengthen — the receiving register's live/dead standing under
`override_set_b`. Together these close the compaction-safety proof
obligation to exactly what it can honestly claim: no resurrection ever,
one-shot suppression is a known and recoverable false-negative risk
inherent to the clear-wins/tombstone design, not an unbounded
correctness gap.

### GC/tombstone behavior

**Override keys with `ov_` prefix (legacy prune):** Legacy
`pruneStaleContexts` only drops `msg:`/`thread:`-prefixed keys past the
7-day horizon. Unknown-prefix keys (including `ov_*`) are kept forever:

- **Permanent tombstones:** every override that is ever compacted while
  dead leaves a permanent `ov_c:` key (~45 bytes, channel context) — this
  is no longer a "harmless, can shrink to zero" cost; it is a durable
  floor kept forever to block stale-replay resurrection. This is the
  direct storage consequence of fixing the CRITICAL above and must be
  budgeted, not treated as free.
- **Live overrides:** an override still live (per `override_set_b`)
  keeps all 3 keys (~138 bytes, channel context) until it becomes dead
  and is compacted down to the tombstone.

**Alternative: nesting under `msg:`/`thread:` prefixes** — confirmed
**state-loss hazard**. Legacy prune would delete overrides at the 7-day
horizon, silently losing active unread markers. Rejected.

### Legacy trim interaction

Legacy `trimContextsToBudget` evicts only `msg:`/`thread:` keys.
Override `ov_*` keys (including tombstones) are never evicted. Budget
analysis by context type, worst case (all overrides still live, 3 keys
each — the tombstone floor only ever *reduces* this cost):

| Overridden contexts | Context type | Live override bytes | With ~10 KiB frontiers | Fits 32 KiB? |
|--------------------|-------------|----------------|----------------------|-------------|
| 50 | Channel (UUID) | ~6.9 KiB | ~16.9 KiB | Yes |
| 100 | Channel (UUID) | ~13.8 KiB | ~23.8 KiB | Yes |
| 150 | Channel (UUID) | ~20.7 KiB | ~30.7 KiB | Marginal |
| 50 | Message (hex64) | ~11.9 KiB | ~21.9 KiB | Yes |
| 100 | Message (hex64) | ~23.7 KiB | ~33.7 KiB | **No** |

At the 100-override cap with every override compacted to its tombstone
floor instead: ~4.5 KiB (channel contexts, 100 × 45 bytes) — well
within budget alongside a full frontier set. The permanent-tombstone
floor from the CRITICAL fix costs storage but is bounded and small; it
does not change the 32 KiB conclusion below.

**Mitigation:** Upgraded clients should compact aggressively (any dead
override, not just baseline-dominated ones) and enforce a cap on active
override count. A cap of 100 channel-context overrides keeps *live*
override budget under ~14 KiB and *tombstoned* budget under ~4.5 KiB,
both within the 32 KiB limit alongside a full frontier set.

### Tie policy evidence: clear-wins vs set-wins

Both tie policies pass all invariants. The choice is a product-semantics
decision:

- **Clear-wins (S == C → read):** If two devices concurrently set and
  clear the same context, the result is "read." Conservative — no
  spurious unread badges. Matches the "I already read this" signal being
  more definitive than the "remind me" signal. Compaction advantage:
  `S == C` states are compactable.
- **Set-wins (S == C → unread):** Concurrent set and clear results in
  "unread." Preserves the reminder intent. Risk: a user who reads on one
  device while another has a stale mark-unread gets a persistent badge
  they can't clear without an explicit action. Compaction disadvantage:
  `S == C` states are live and cannot be compacted.

**Recommendation:** Clear-wins. A false negative (missing badge) is
recovered by re-marking unread. A false positive (badge that won't clear)
is more frustrating. This matches Slack's behavior: reading anywhere
clears everywhere. The compaction advantage further favors clear-wins.

**Pre-existing false-negative risk (independent of compaction).** Under
clear-wins, a stale explicit clear (`RegB(0,1,0)`, no compaction
involved) merging into a device with a fresh concurrent set
(`RegB(1,0,30)`) already produces `RegB(1,1,30)`, tied, suppressed —
verified directly by evaluating `merge_reg_b`/`override_set_b` on those
two registers with no `compact_b` call anywhere in the path. The
cross-device tombstone-suppression finding (I5d, "Compaction behavior"
above) is the same tie shape reached via a different route: a
baseline-dominated *dead set* (never explicitly cleared) that gets
compacted to a `C`-ceiling tombstone, which is then globally comparable
in a way its pre-compaction, frontier-relative death was not. Compaction
widens the set of histories that can reach the tie, but clear-wins
already accepted this one-shot, re-mark-recoverable false-negative shape
as its stated tradeoff.

### Multi-slot union

Production splits blobs across up to 8 slots (`READ_STATE_MAX_SLOTS`).
`mergeReadStateEvents` merges all slots with per-context `max()`. Override
sibling keys are individual context entries and follow the same merge path.

Confirmed: splitting a published blob across 2 slots and delivering each
separately produces the same final override and frontier state as
delivering the full blob, regardless of delivery order.

## Mutation harness

6 mutants, all caught with recorded counterexamples:

| Mutant | Rule dropped | Counterexample |
|--------|-------------|----------------|
| M1 | Baseline dominance check | `RegB(1,0,10)` at frontier=100: correct=inactive, mutant=active (stale set persists) |
| M2 | `max(S,C)+1` counter bump | After set→set→clear: correct `RegB(2,3,10)` (clear wins), mutant `RegB(2,1,10)` (set persists) |
| M3 | Tie policy | `RegB(1,1,10)` at frontier=10: clear-wins=False, set-wins=True |
| M4 | Tombstone-floor compaction (delete-on-dominance revert) | `RegB(3,0,10)` at frontier=20 compacts to `None` (vs. tombstone `RegB(0,3,0)`); local set+clear reuses counters from zero; delayed stale replay resurrects — `final_reg=RegB(s=3,c=2,b=20)`, `override_is_set=True` (reproduces Thufir's pass-3 CRITICAL) |
| M5 | uint32 value range | Value 4,294,967,296 rejected by legacy sanitization |
| M6 | Componentwise-max merge | LWW delivery-order-dependent: convergence breaks under permutation |

Each mutant is injected into the model via DeviceB subclass (M1, M2, M4,
M6) or direct function evaluation (M3, M5), then the applicable invariant
suite is rerun. M4 reverts to the pre-fix delete-on-dominance compaction
rule and directly reproduces Thufir's pass-3 CRITICAL resurrection
witness — the exact `RegB(3,0,10)` → `None` → counter-reuse → stale
replay → `RegB(3,2,20)`,`override_is_set=True` sequence — with a
fallback to the directed deep-history cube (`test_deep_history_compaction`)
if the hand-built scenario doesn't trigger under a given tie policy.

## Recommendation

**Candidate B (two grow-only counters + baseline) with clear-wins tie
policy.**

Evidence:

1. **Legacy safety:** B's sibling keys survive legacy rewrite; A's
   top-level field is erased. Hard blocker for A — no migration path
   tolerates a single legacy device.
2. **Identity-free:** B needs no client_id for correctness; A's
   tiebreak creates a reinstall fragility.
3. **CRDT properties:** Both pass all merge invariants. B's
   componentwise max is simpler and more standard.
4. **Bytes:** B at 3 live keys costs 138 bytes/context (channel UUID) to
   243 bytes/context (thread hex64); a dead override compacts to a single
   ~45-80 byte tombstone key instead. Cap of 100 overrides stays within
   32 KiB budget for both live and tombstoned cases.
5. **Compaction:** B supports safe policy-aware compaction — no
   resurrection, ever (proved structurally, not just over a bounded
   cube). Clear-wins allows compacting `S == C` states (set-wins does
   not). Cross-device delivery of a tombstone can one-shot suppress an
   unrelated device's concurrent fresh set whose counters are at or
   below the tombstone's ceiling; this is recoverable by re-marking and
   is the same false-negative shape clear-wins already accepts for a
   stale explicit clear with no compaction involved (see "Tie policy
   evidence").
6. **Tie policy:** Clear-wins avoids persistent false-positive badges
   and enables more aggressive compaction.

## Honest limits

- The model enumerates bounded abstract operations, not real encrypted
  NIP-59 payloads or relay replacement semantics.
- Counter values in the general BFS explorer are bounded by its exploration
  depth (max ~4 via BFS depth 4); the directed deep-history cube
  (`test_deep_history_compaction`) reaches counter values up to the stale
  parameter range (0-3) plus post-compaction action sequences, covering the
  ~9-transition witness the BFS explorer cannot structurally reach. Real
  uint32 overflow/wrap is tested only via the legacy sanitization mutant (M5).
- The BFS explorer (I5/I5c) checks compaction safety over reachable
  multi-device histories up to depth 4, but its own terminal-state
  compaction check (`check_compaction_safety`) only merges a device's
  compacted register with its *own* pre-compaction snapshot — it does
  not, by construction, exercise an unrelated device's independently-
  live concurrent register. `test_cross_device_compaction_suppression`
  (I5d) covers that shape directly but over a hand-parameterized cube,
  not the full BFS state space; the accompanying
  `test_tombstone_merge_monotonic` lemma is what extends the
  no-resurrection guarantee beyond the cube's specific points.
- Two contexts are modeled. Production users may have hundreds of contexts,
  but the CRDT properties are per-context — cross-context interactions are
  limited to the shared byte budget (tested via trim/prune interaction).
- Multi-slot behavior is confirmed via split+merge convergence test, but
  the split algorithm itself (`splitContextsIntoBudgetedSlots`) is not
  modeled. Implementation-level testing is needed for slot placement.
- The model assumes eventual delivery (all blobs eventually reach all
  devices). Permanent message loss is not modeled.
- Byte sizes are computed from JSON serialization of realistic key names.
  Actual encrypted blob overhead (NIP-59 envelope, relay metadata) adds
  to the total but does not affect the 32 KiB plaintext budget.
