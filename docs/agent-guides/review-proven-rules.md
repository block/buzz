## Review-Proven Rules

These rules distill the recurring findings from the last 25 PRs' review
threads — 53% of substantive review findings were repeats of the clusters
below, and reviewed PRs averaged ~5 review rounds. A second, independent
mining pass over 71 agent-review rooms (303 findings, Aug 18–29) confirmed
the same clusters and measured how often authors actually fix each class
once flagged: test-seam binding and unbounded-resource findings were fixed
**100%** of the time, swallowed-error findings **90%**, stale-state races
**70%** — these are not style opinions, they are defects authors agree
with on sight. Apply the rules **before writing code**; each cites the
PRs where reviewers litigated it.

1. **Every caught failure must leave a durable retry record or propagate.**
   Never catch-log-and-return-success (opt-out revocation permanently
   abandoned, PR #6269), never convert a terminal failure into an
   authoritative success/empty result (cold-history `error` → `success`
   with `[]`, PR #7013), and never delete the durable journal an operation
   depends on before its retry has actually succeeded (PR #6269). If a
   partial failure can orphan committed state (installations, endpoints),
   schedule its cleanup/renewal durably (PRs #6269, #6996, #7013).

2. **Fence async results by generation; clear derived metadata on every
   removal path.** A completing in-flight probe or fetch must verify it is
   still the newest before writing its result (stale login-shell probe
   recached a false-negative PATH, PR #6904). Provenance/ownership metadata
   attached to synthetic state must be updated or cleared on *all* paths
   that remove or refresh that state — typed deletion, toolbar removal,
   profile/name refresh; enumerate the paths and test each (PR #6956 burned
   4 rounds on this one class). Backfill and live subscriptions must
   overlap — a gap between a finite history REQ and the live subscription
   silently drops events (PR #3995); a retired chunk must not keep a stale
   scope fence (PR #6996). (PRs #3995, #6904, #6956, #6996)

3. **Regression tests must bind the production seam and be falsifiable.**
   See "Review-Proven Test Standards" in [TESTING.md](../../TESTING.md) for the
   full rule — in short: a guard whose removal doesn't fail any test
   protects nothing; bind regression tests to the production code path,
   not test-only helpers. (PRs #6807, #6980, #6996, #7013)

4. **Bound every resource, loop, and process tree.** Cap captured
   output (unbounded discovery temp files exhausted disk and overran the
   deadline, PR #6904). Containment failures are errors, not warnings — a
   tolerated Job Object creation failure or a `setsid` escape leaks whole
   process trees (PR #6904). Retry/re-subscribe loops need backoff and a
   terminal state: a persistent failure must not self-amplify into an
   unbounded refresh loop (PR #6996), and check zero-delay edge cases
   (`remainingMs()==0` selected the wrong fallback window, PR #6996).
   (PRs #6904, #6996)

5. **One user action = one atomic persist.** Implementing a single user
   commit as N independent durable writes leaves torn state on partial
   failure (theme "Set" as three independent notifier persists, PR #6944;
   relay-commit vs. local-save recovery gap, PR #6269). Persist one
   snapshot, or order the writes so every prefix is consistent and the
   remainder is durably retried per rule 1. (PRs #6269, #6944)

6. **A guard that hides the only recovery affordance is a functional
   failure.** Before adding a visibility predicate or state fence, ask:
   if the state it assumes goes wrong, does the user still have a way
   back? A fence that permanently suppresses "jump to latest" after a
   bounded correction fails strands the user silently — two reviewers
   flagged this independently (PR #6807).

7. **Audit assistive semantics on every new visual component.** The
   agent-review lanes flagged accessibility defects on 44 findings across
   the Aug 18–29 window — the second-largest cluster — and authors fixed
   the concrete ones (duplicate VoiceOver stops on native controls,
   actionable labels owned by two widgets at once, PR #6680; missing or
   decorative-leaking semantics on new UI, PRs #6611, #6702, #6885, #6905,
   #6908). New UI ships with: one owner per actionable label, no duplicate
   screen-reader stops, and explicit semantics for every interactive
   element. (PRs #6611, #6680, #6702, #6885, #6905, #6908, #6980)

8. **Every input modality is a first-class seam.** Keyboard, pointer, and
   hotkey paths must not silently diverge: `Shift+Space` treated as plain
   `Space` because the guard omitted `shiftKey` (PR #6862), keyboard
   ownership not released on blur, modifier keys dropped on the non-mouse
   path (PRs #5958, #6793, #6860, #6908, #7006). When adding an input
   handler, enumerate the modalities that can reach it and test the
   non-primary ones — that's where the defects were. (PRs #5958, #5972,
   #6793, #6860, #6862, #6908, #7006)

---
