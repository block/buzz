---
status: Proposed
date: 2026-08-13
issue: launchpad-26/buzz#63
decided_in: launchpad-26/buzz#63
supersedes: none
---

# ADR-0006 — Secret-scanning engine for the fork, and where its allowlist lives

## Decision

The fork's secret-detection engine is **gitleaks**, invoked both on the PR-diff path and
on a full-history scan on the scheduled path, per #62. Detection rules and allowlist live
together in a single `.gitleaks.toml` at the repo root, extending gitleaks' default
ruleset (`useDefault = true`) with fork-specific rules for the material stock rulesets do
not know about: Nostr `nsec`/hex private keys, `BUZZ_PRIVATE_KEY`, Unix/glibc crypt hashes
(`$1$`/`$5$`/`$6$`/`$y$`), and `BUZZ_S3_*` / Postgres-URL-with-embedded-password shapes.

The allowlist is **hand-maintained TOML `[allowlist]` / per-rule `allowlist` blocks in
`.gitleaks.toml`, each entry carrying a `#` comment stating why it's safe** — the same
shape as the `.intersect/sadscan.yaml` precedent #62 points to. Gitleaks' other allowlist
mechanism, an auto-generated `--baseline-path` JSON snapshot of accepted fingerprints, is
explicitly **not** used: it has no field for a reason, and #63's own decision drivers flag
it as "a loaded footgun: regenerated carelessly, it silently accepts a real finding." A
TOML block requires a human to type a reason; a regenerated baseline requires nothing.

This decision covers the CI-side detection engine only. It does **not** decide whether
GitHub's native secret scanning + push protection gets enabled — that is a repository
setting requiring admin, tracked separately (see Context).

## Context

No secret detection runs on this fork today. `.intersect/sadscan.yaml` configures a
scanner driven by Block-internal CI that never triggers here — `grep -rn
"sadscan\|intersect" .github/ Justfile` returns nothing, confirmed in #62's own evidence.

Three options were considered and rejected in #63 as filed, on grounds independent of
anything tested below:

- **TruffleHog** — rejected because its core feature, live credential verification, sends
  candidate secrets to third-party APIs (documented example: an AWS credential detector
  calling `GetCallerIdentity`). For a repo whose likeliest findings are live host and
  database credentials, that turns a detector into an egress channel. The AGPL-3.0 licence
  was noted for the record, not the basis for rejection.
- **GitHub secret scanning + push protection as the sole answer** — rejected as the *only*
  answer because enabling it needs repository admin, which this work does not have. It
  remains valuable as a complementary, *preventive* control (see below).
- **Hand-rolled grep/regex script** — rejected for no history-walk, no baseline/allowlist
  semantics, and making the cohort maintainer of a bespoke scanner.

That left gitleaks as the only candidate meeting every driver (local, no network egress,
history-capable, TOML rules and allowlist reviewable in-repo, one local command). Rather
than accept that conclusion on paper, it was run directly against this repository before
being decided:

- **Full-history scan**: `gitleaks detect --source .` completed in **12.4s** over 5,537
  commits / 102.55 MB — comfortably inside #62's 3-minute PR budget, and this is the
  *slower* of the two scan modes.
- **False-positive rate, measured, not assumed**: that same scan surfaced **65 findings**
  on real repo history, nearly all plausible false positives at a glance (test-fixture
  tokens, example JWTs, an example `nsec1abc123def456` in documentation). This is direct
  evidence for the allowlist requirement, not a restatement of the issue's prose.
- **Default-ruleset gap, measured**: a fixture file was staged in a throwaway scratch repo
  containing four planted secrets shaped like this cohort's real material — a `BUZZ_PRIVATE_KEY=nsec1…`
  value, a `$6$rounds=5000$…` crypt hash, `BUZZ_S3_ACCESS_KEY`/`BUZZ_S3_SECRET_KEY`
  values, and a `postgres://user:pass@host/db` URL — then scanned with `gitleaks protect
  --staged` using gitleaks' default rules only. **Only 1 of 4 was caught**, and only
  incidentally, via the generic high-entropy rule rather than anything Nostr-aware. The
  hash, the S3 credentials, and the Postgres URL all passed through undetected. This
  confirms #63's decision driver ("must cover material stock rulesets do not know about")
  is not a hypothetical — it is the default behavior.
- **Custom-rule feasibility, measured**: a minimal `.gitleaks.toml` with four fork-specific
  rules was written and re-run against the same fixture. The S3-credential and
  Postgres-URL rules caught their targets on the first pass. The Nostr and crypt-hash
  rules did not, on the first attempt — the planted fixture's `nsec` string wasn't valid
  bech32 (it contained characters bech32 excludes), and the crypt-hash regex didn't
  account for glibc's optional `rounds=N$` segment. Both are fixable, ordinary regex
  iteration — the real cost is that iteration itself, not a blocked path. Writing that
  ruleset correctly is scoped to #67, not this ADR; this ADR only needed to confirm the
  gap is closeable, which it is.

**On GitHub's native secret scanning + push protection**: #63 as filed rejected this as
the sole answer because it "needs repository admin, which this work does not have."
That's true of the account doing this work, but the cohort's PM does hold admin on this
repo. That reopens a door #63 and #65 (a related, still-open ADR, about the *audit
workflow's own CI token* — a different question, still admin-less either way) both treated
as closed. Push protection is the only *preventive* control available anywhere in this
PRD — everything gitleaks does here is detective, catching material after it's already
committed. Requesting the PM enable it is now a live, actionable ask, tracked under #72
("request and verify the admin-only repository security settings"), not a permanently
blocked item. It does not change this ADR's engine choice: GitHub's native scanning still
doesn't cover `$6$` hashes or Nostr keys without custom patterns (themselves admin-gated to
configure), and isn't reproducible locally by an agent the way gitleaks is. The two are
complementary, exactly as #63 as filed anticipated — this ADR just confirms the admin path
is reachable via a different person, not that it changes what CI runs.

## Consequences

**Good.** One engine, one rule format, rules and allowlist reviewable in a PR diff
alongside the code they protect. A local `gitleaks detect`/`gitleaks protect --staged` run
reproduces CI exactly — verified, not assumed, since that is literally what was run above.
Nothing leaves the runner: no third-party API calls, no verification step to accidentally
enable later. Measured startup cost is low enough that #62's 3-minute budget is not a
concern for this piece of the audit.

**Bad, stated honestly.** The 65-finding run against real history is concrete proof that
every finding needs human triage — a green run only means "no *new* uncaught pattern,"
never "no secrets." The allowlist is the soft spot: an over-broad entry silences detection
without looking like it changed anything, which is why entries must be scoped to a rule
or a specific match, never a whole file or path, and each must carry a reason. Writing
rules that correctly match this cohort's real material (valid bech32 charset, glibc's
optional `rounds=` segment, and whatever else surfaces) is real, ongoing work — this ADR's
own first-attempt rules needed a second pass, and #67's eventual ruleset should expect the
same. And because this is a regex-only engine with no liveness signal, a genuinely novel
secret shape not yet written into a rule will pass silently — planted-fixture proof per
issue, not a one-time check, is why #62 demands it as an ongoing success criterion rather
than a launch-day checkbox.

## Provenance

This decision was made directly while working #63 — `issue` and `decided_in` point to the
same place.

Verified live, on this machine, against this repository, rather than assumed from #63's
filed text:
- gitleaks 8.30.1 full-history scan timing and finding count (5,537 commits, 102.55 MB,
  12.4s, 65 findings) — reproducible with `gitleaks detect --source . --no-banner -v`.
- The default-ruleset gap against four planted, cohort-shaped fake secrets in an isolated
  scratch git repository (never committed to this repository) — reproducible with
  `gitleaks protect --staged --no-banner -v` against a staged file containing an
  `nsec1…` value, a `$6$rounds=…` hash, `BUZZ_S3_ACCESS_KEY`/`SECRET_KEY` values, and a
  `postgres://user:pass@host/db` URL.
- That a minimal custom `.gitleaks.toml` closes part of that gap immediately, and that the
  remaining gap is a regex-correctness problem, not a capability gap.

Not verified independently: TruffleHog's and GitHub secret scanning's own detection
behavior against the same fixture — their rejection rests on #63's documented licence,
egress, and privilege grounds, which were not re-tested, only gitleaks' fitness was
verified hands-on.

This decision directly shapes #67 (the secret-detection task under #62, which owns writing
and hardening the actual `.gitleaks.toml` ruleset — the draft rules used here are a
starting point, not the final ruleset) and #72 (which should now carry the specific,
actionable ask that the PM enable GitHub push protection, rather than treating it as
blocked).
