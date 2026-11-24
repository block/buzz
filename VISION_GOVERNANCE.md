# 🏛️ Buzz Governance: Your community, your rules for change

> Dana asks her agent to fix a rounding bug in `ledger`. The agent pushes a branch, opens a pull request, and spins up `#fix-rounding` to work it. The conversation comes alive: the community's CI posts green, the secret scan posts clean. A rules card pinned at the top says what's left. One approval from someone who isn't Dana, and one from `#security`, because `ledger` carries the `payments` label. Sam approves. Ines from security approves. Dana hits **Land**. The relay checks every rule one last time and moves `main`. That evening Dana revokes the agent's grant; the job is done.
>
> Fourteen months later an auditor asks who approved the rounding change. `#fix-rounding` is long archived. Sam has moved teams. An admin pulls the receipt: the exact commit, the agent and the human it worked for, both approvals, the check runs, and the rules in force that day. She exports a signed copy the auditor can verify offline. Nobody had to remember anything.

[VISION_MODERATION.md](VISION_MODERATION.md) makes a community governable as a conversation. This doc makes it governable as a body of work. Moderation decides who may speak. Governance decides what may change: who can move a ref, what proof it takes, and what record it leaves behind. The relay provides the mechanics. The community provides the judgment.

There is no separate enterprise version of governance capabilities. A three-person open source project and a regulated company use the same primitives. The company just turns more knobs. "Main needs one approval from someone who isn't you" is good hygiene for a hobby project too. If a governance feature only makes sense with corporate sign-in, it belongs behind the identity issuer seam, not in core.

*This is the governance contract we intend to build. See [Status](#status).*

---

## Three Levels

Git in Buzz is governed at three levels, each owned by the people who already own that space:

| Level | Who sets it | What it decides |
|---|---|---|
| **Community** | Owners and admins | The floor every repo stands on, labels, agent limits, trusted checks, retention |
| **Repo** | The repo channel's admins | Rules on top of the floor, and who reads, writes, and manages |
| **Change** | Everyone working on it | The evidence: reviews, checks, discussion, and the moment it lands |

Rules flow down. Evidence flows up. A level can add to what's above it, never subtract. Authority lives at the top two levels; a change only carries evidence.

---

## The Community: Setting the Floor

Community owners and admins write rules that apply to every repo, existing and new, and that no repo admin can weaken. In plain terms, a floor looks like this:

```
every repo        main: no force-push, no delete
                  any push: reject detected secrets
label payments    main: 2 approvals, 1 from #security
                  main: check "ledger-ci" from ci-bot must pass
agents            grants name their repos, last at most 7 days
```

*(Illustrative. The wire format belongs to the companion protocol proposal.)*

**Labels** are how one rule reaches many repos. An admin puts `payments` on a repo and every `payments` rule applies to it immediately. New repos can start with default labels, so nothing is ungoverned on day one. Anyone on Nostr can write a label; only labels accepted from community admins or integrations they explicitly authorize affect this community's rules. The relay signs the current set, meaning "this community accepted this label," not "the universe agrees this repo is compliant."

**Trusted checks** are named, not assumed. The community decides which CI identities count. A green check from anyone else is just a message. A trusted check counts as a check, never as a human review.

**Secret scanning** runs at the door. Detected secrets are rejected before incoming code becomes visible to anyone else.

**Agent limits** cap what any member can hand to an agent or integration: which repos, which actions, how long, and whether widening a grant needs an admin.

Every change to the floor is a signed command from a named admin, checked against their role and recorded, the same way moderation actions are. The relay publishes the current rules to the people allowed to read them.

One layer sits above the community: the **hosting operator**. "Repos here can never be public" is a deployment constraint pinned in reviewed configuration, not an admin's call. A self-hoster is community and operator at once. A managed host is only the operator, and its authority over a community's rules stops there.

---

## The Repo: Building Up

A repo is bound to a channel, and the channel's roles are the repo's permissions. Guests read. Members write. Admins manage: default branch, protections, and which rules the repo adds. A private channel makes a private repo, and everything about it follows: issues, status events, and labels are visible only to the channel's members.

Repo admins add rules on top of the floor. They can require a second reviewer, protect a release branch, or name code owners for a directory. They can't remove the community's review, turn off secret scanning, or loosen the agent cap. Code owners add required review without granting repo access, and a change can't redefine its own required reviewers. When rules overlap, every applicable rule must pass. Nothing gets "overridden by the stricter one"; they all just apply.

The rules card on every pull request shows the combined result and where each rule came from, so nobody has to guess whether a requirement is the repo's preference or the community's floor.

---

## The Change: Where It Lands

Every ref change goes through the same gate and leaves a receipt, whether it's a merge, a direct push the rules allow, or a deleted tag. A proposed code change is represented by a pull request. Its work channel gives the discussion a home, not additional authority: the repo channel, or a room spun up just for the work, as in [VISION_PROJECTS.md](VISION_PROJECTS.md). That channel is a workspace, not a permission. Joining `#fix-rounding` grants nothing on `ledger`; who can read, push, and approve always comes from the repo channel and the community floor.

The work can happen in another room; its permissions stay with the repo. Reviews, checks, and code shown from the pull request remain visible only to people allowed to read the repo, wherever Buzz displays them, and stop showing to anyone who loses repo access. Ordinary chat is different: it has the room's audience, and Buzz can't make text someone pastes by hand inherit the repo's privacy. Buzz-generated diffs and check details never fall back to the room's audience.

Governance adds two things: evidence that means something, and one door.

**Evidence names exactly what it approved.** Every approval and check result records the commits, repository, and target branch it saw. A yes in one repo never counts in another. Rules decide what still counts: how many approvals, from whom, whether a new push clears them, and whether checks must pass on the merge result. Missing, pending, or unavailable required checks block the change. Any account that carries, or is required to carry, an owner attestation ([NIP-OA](docs/nips/NIP-OA.md)) can't fill a required human approval. In a community backed by an identity issuer ([NIP-FI](docs/nips/NIP-FI.md)), the issuer's word counts too: a key it admitted as a workload or an agent never fills a human approval, and the community can require every human approval to come from someone the issuer vouches for as a person. Communities without an issuer keep the attestation rule alone. When the rules ask for independent review, nobody approves their own work: an agent counts as its owner, and "their own work" means every key that pushed or signed the pull request's commits.

**One door, one decision.** Whoever lands a change, whether Dana, her agent, or a merge coordinator, goes through the same gate. Agents and CI do the compute. The relay decides, in one step, with current authority and current evidence. A change can land only under authority still valid at that moment. You can use another client. You cannot use another entrance to skip the rules.

**Every decision leaves a receipt.** When the relay says yes, it records what changed, who acted, for whom, under which rules, on what evidence, and what actually happened. When it says no, it records that too, along with the rule that said it. Members see the reviews, checks, and outcome in the channel. The full receipt lives in the community's record.

When the change lands, its work channel can archive. The conversation stays as the story of why the code exists. The receipt stays as the proof.

---

## Agents: A Grant, Not a Key

An agent reaches a repo only through its own membership in the repo channel, never through its owner's. Where the community requires it, the agent also needs a grant from you: these repos, these actions, until Friday. A grant only narrows. It never lets an agent see a channel it isn't in, and it can never exceed your own authority or the community's cap. The grant lives in the community, not in a tag the agent carries, so revoking it bites on the next request. An agent never holds more than its owner currently does. When you lose access to a repo, your agents lose it on the same schedule, grant or no grant.

The agent's key is still its own key, and **handing over the key doesn't hand over the agent**. Anyone holding it can sign as the agent, and anyone can write a new owner attestation. But a community records an agent's first owner and keeps it. A different owner is refused, and whoever holds the key acts on the original owner's account. To give someone an agent, give them a new key.

---

## The Record

Membership gates collaboration. Owners and admins keep the record. Policy gates changes.

- **It outlasts the work.** Archiving or deleting a work channel, removing a contributor, or replacing a rule cannot erase the record while a retention rule still covers it.
- **It stays as private as the work.** Keeping a record never publishes it. Former members lose access, same as today.
- **It's more than merges.** Refusals, reads, grants, rule changes, and every look at the record itself.
- **It travels with the community.** Admins can export a signed copy anyone can verify offline. The copy belongs to the community, every export is recorded, and the formats are documented so the record can be restored elsewhere.

Admins reading a private repo's retained history is a new reach, beyond today's "you see the channels you're in." We name it, and every use is recorded.

---

## Identity

Your key is yours. The relay never sees it, and an identity assertion doesn't let its issuer sign as you. The community governs its shared work.

In a community backed by an identity issuer ([NIP-FI](docs/nips/NIP-FI.md)), the issuer vouches for who you are and for how long. The relay checks that voucher when you connect and on every git request, and no session outlives it. Corporate identity says who you are and when that stops. The community says what you may do.

A "no" from inside Buzz (a ban, a removal, a revoked grant) takes effect on the next request. A "no" from the issuer lands when the issuer stops vouching for you. Once it stops issuing, you can't get back in after your current voucher runs out, and the community sets how long a voucher can live. A disconnect from the issuer can cut that short wherever it's applied, but it isn't the bound. Whether a write already in flight can still land is a separate promise: only under authority valid at its final acceptance.

A governed community records which person stood behind each change. If you reuse that key on public Nostr, it's linkable to that identity. Use a separate key per governed community if that matters to you.

Public contributions stay portable, and your reputation on open projects travels with your key. Private records stay with the community that kept them. It can choose to release what it wants, as an export or a public attestation that you shipped good work.

---

## Honest Edges

**Signatures are not an audit.** Signatures establish who signed. Chained records make alteration detectable against a trusted checkpoint. Retention, completeness, and recovery still need operating controls, and an auditor will ask about those.

**Corporate revocation is as fast as the voucher.** An issuer-backed community trades instant revocation of corporate identity for a stateless front desk with no identity registry. A deployment that needs a tighter bound shortens how long a voucher can live. The time from your directory to your issuer is the issuer's promise, not ours.

**Group sync is young.** Teams come from your directory and live as channels. Until the issuer syncs groups, group changes reach Buzz only when someone makes them.

**Fail closed.** A required rule the relay doesn't understand means no. Never skip, never warn and continue. Mixed-version rollouts will feel that, on purpose.

**Two roles, not three.** Owners and admins set the floor and keep the record. A read-only auditor role is a policy change later, not a rewrite, the same call moderation made.

---

## Status

This is the proposed contract, not what ships today. Git hosting, channel-bound repos, NIP-34 pull requests, NIP-FI identity, and moderation's signed commands exist. Channels created per branch are a convention today, not a feature, and governance doesn't depend on them. Today a repo is managed by its owner and named maintainers; moving that to channel admins is part of this work. Governed repos take changes as pushed branches with a NIP-34 pull request. Patch events and today's `require-patch` protection don't carry over. Community floors, labels, trusted checks, agent grants, the single landing decision, receipts, retention, and export are still to build. A companion protocol proposal will define the wire, composing NIP-29 roles and NIP-34 repo identity, so another implementation can build the same contract without our permission.

---

## The Point

Your key is yours. This community's rules apply here. A maintainer can add a review, but cannot remove the community's review. An agent can do the work you give it, within the authority you still have. When a change lands, the community keeps the code, the evidence, and the decision together. You can use another client. You cannot use another entrance to skip the rules.

---

*Buzz 🐝: your community, your rules for change.*
