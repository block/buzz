# Environments

Where Buzz runs for this cohort, what each place is for, and what must hold true no matter
which one you are looking at.

This document records a target and a set of invariants. It does not build any of it, and
it does not settle anything an open decision owns — where a choice is open it says so and
links the issue.

---

## How to read this

Every claim below carries exactly one status marker, read as defined in
[VISION.md § How to read this](VISION.md#how-to-read-this). The legend lives there and is
not restated here.

---

## The environments

| Environment | Purpose | Status | Tracking |
|---|---|---|---|
| Local development (`localhost`) | Day-to-day work against a relay on the developer's own machine | `IMPLEMENTED` | — |
| Local VM harness | Rehearse Host-to-community binding and membership gating before touching the VPS | `OPEN` | [#17](https://github.com/launchpad-26/buzz/issues/17), [#19](https://github.com/launchpad-26/buzz/issues/19) |
| Cohort VPS | The shared internet-facing relay | `OPEN` | [#21](https://github.com/launchpad-26/buzz/issues/21), [#22](https://github.com/launchpad-26/buzz/issues/22) |
| Contributors' machines | Where relay-initiated agent work executes | `OPEN` | [#43](https://github.com/launchpad-26/buzz/issues/43) |

No hostnames. `AGENTS.md` §8 forbids private hostnames in tracked files, and the VPS
hostname is not decided anyway.

The status markers above cover only the rows present when this section was written. A
new environment is a new row, and it carries its own status marker and its own tracking
issue rather than inheriting these.

---

## What each one proves

| Environment | What it proves that the others cannot |
|---|---|
| Local development (`localhost`) | The relay and clients run at all, with no network, DNS or TLS variable in the way. `IMPLEMENTED` — this is how the cohort has worked so far. |
| Local VM harness | That Host-to-community binding and membership gating behave as expected, rehearsed somewhere an internet-facing host does not yet depend on the outcome. `OPEN` — [#19](https://github.com/launchpad-26/buzz/issues/19). |
| Cohort VPS | That the relay works as a shared service reachable by two different people on two different machines, rather than only a local one. `OPEN` — [#2](https://github.com/launchpad-26/buzz/issues/2). |
| Contributors' machines | That a relay-initiated agent can execute somewhere other than the relay itself, on infrastructure the cohort does not own. `OPEN` — [#43](https://github.com/launchpad-26/buzz/issues/43). |

The section-level markers above cover only the rows present when this section was
written. A row added later does not inherit one — give it its own status marker and its
own link to evidence.

---

## What must never differ between them

Two properties of the software today, sourced to
[#2](https://github.com/launchpad-26/buzz/issues/2)'s Evidence section. Both are marked
`IMPLEMENTED` — they are true of the code now, not decisions anyone is waiting on.

**The relay derives its community from the request `Host` header, so the hostname
serving each environment must exist in the `communities` table or every connection is
rejected.** `IMPLEMENTED` — [#2](https://github.com/launchpad-26/buzz/issues/2): "The
relay resolves its community from the request `Host` header. A deployed hostname absent
from the `communities` table rejects every connection." Every environment above needs its
own serving hostname seeded as a community before anyone can connect through it.

**The desktop client assumes `wss://` for any scheme-less host, so a plain `ws://`
deployment is typed wrong by users.** `IMPLEMENTED` — [#2](https://github.com/launchpad-26/buzz/issues/2):
"The desktop client assumes `wss://` for any scheme-less host, so a plain `ws://`
deployment will be typed wrong by users." Any environment served without TLS needs its
`ws://` scheme stated explicitly wherever the address is shared, or users will be misled
into an address the client will not reach.

These two markers cover only the two properties stated when this section was written. A
property added later carries its own marker and its own link to evidence.

---

## Adding to this document

> Additions arrive by pull request against `launchpad`. Every new claim carries a status
> marker and a link to its evidence. Anything not yet true is an issue, not a line here —
> see [`AGENTS.md` §2](AGENTS.md). Append within a section rather than renumbering
> headings, so links from issues and the handbook keep resolving.

A new environment is a new row in [The environments](#the-environments), and it carries
its own status and tracking issue.
