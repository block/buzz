# Reviewing mobile changes

Review mobile PRs, and relay or push-gateway changes the mobile app relies on,
against these principles, alongside the [Reviewing](../AGENTS.md#reviewing)
guidance and [Review-Proven Rules](../AGENTS.md#review-proven-rules). A
regression against them is a concrete defect: state the mechanism (code path,
input or state, and the wrong result) and its consequence for the user, as for
any blocking finding.

1. **Land Pareto improvements.** A change that's better somewhere and worse
   nowhere should land. For a feature, its bugs must stay inside the feature,
   and having it must beat not having it. It's not a Pareto improvement if it
   regresses performance, reliability, codebase or architecture quality, or
   makes the UI harder to understand. Authors state this case in the PR
   description: what gets better, and why nothing existing gets worse, or
   which tradeoff they accept and why.
2. **No hangs or stutters.** Keep expensive work (decoding, crypto, parsing,
   big sorts) off the UI isolate. Avoid needless rebuilds, especially during
   animations and keyboard motion.
3. **Use the network carefully.** No polling when live updates exist. Fetch
   only what changed. Survive slow and flaky links.
4. **Don't clog the pipe.** Work the user isn't waiting for must never delay
   work they are.
5. **Reliability means the app stays usable.** A rare crash with no data loss
   beats getting stuck broken, laggy or spinning. Data loss is always a
   blocker.

## Examples

- Decode off the UI isolate: PR #8669; native crypto: PRs #8654, #8657.
- Needless rebuilds: unchanged state published on a timer (PR #8695);
  identity-unstable theme objects (PR #8928); work in `build()` (PR #8929);
  covered pages rebuilding during keyboard motion (PR #8941); over-broad
  watches (PR #8672).
- Network: polling replaced by live triggers (PR #8910); fetch from last seen,
  not a fixed horizon (PRs #8901, #8903); hung connects need timeouts
  (PR #8878).
- Clogged pipe: duplicate downloads (PR #8879); unneeded snapshot downloads
  (PR #9093); batches too big for slow links (PR #8901); abort stalls, not
  slow progress (PR #8881).
- Usability: keep loaded content visible while reconnecting (PRs #8864,
  #9100); isolate subsystem failures (PR #8610).
