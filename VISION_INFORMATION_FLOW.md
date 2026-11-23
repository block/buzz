# Vision: Information flow through agents

Buzz agents act with permissions that belong to their owner. Another person can prompt the agent, while the agent can still read its owner's private information in Buzz. That is a confused-deputy problem. Prompt injection makes it easy to exploit, but the root problem is that an untrusted request can reach an agent that holds more authority than the requester should receive.

Buzz solves the narrow part of this: an agent must not move private information to a broader audience unless its owner decided it. It does not try to solve general prompt injection.

The goal is to **control the flow of information between people, with the agent as the conduit**, using the access control Buzz already has: channel membership. [Practical information-flow for Buzz agents](docs/practical-information-flow-for-buzz-agents.md) describes a broker design for the same problem. Buzz follows the channel-based design below instead (see [Alternatives considered](#alternatives-considered)).

This document describes direction. Rules that the relay does not enforce yet are listed in [Rollout](#rollout).

## The model

Treat the agent as a conduit. Two sets describe it:

- **W** is whose content goes in: every channel the agent can read, as a member or a guest.
- **R** is who can get information out: everyone who can read a channel where the agent can write.

Information is safe when, for every channel *c* in W, everyone in R can already read *c*. A private channel in W with a reader in R who is not one of its members is a leak path.

Channel roles decide W and R. The roles that can post (member, bot, admin and owner) can read and write in a channel, and a guest can only read ([Announce channels](NOSTR.md#announce-channels)). So:

- When an agent joins a channel, that channel becomes part of W.
- When an agent has a role that can post in a channel (not guest), everyone who can read that channel becomes part of R.
- An open channel is readable by the whole community, so the first open channel where the agent can post makes R the whole community.
- Reading an open channel is always safe: its content is already public.

**If the agent's owner controls which channels the agent joins, and whether it is a guest or a member in each, then the owner controls W and R, and so controls how information flows between people through the agent.** The relay already enforces channel membership and roles. The rest of this document closes the paths where the owner does not have that control.

## Where the owner loses control

Each of these paths changes W or R without the owner. Checked against `main` at `5b1698ec2`, and the DM rows at `2e3bebe9b`.

| Path | Who does it | Effect |
|---|---|---|
| Someone adds the agent to a channel. In a private channel, any member who can post can do it. In an open channel, any poster can, and so can a non-member when `join_role = member`; the agent joins at `join_role`, member by default. The agent's `channel_add_policy` defaults to `anyone`, and the agent sets it itself with kind 10100, so it can loosen it at any time. | Members of that channel, or in an open channel with `join_role = member`, anyone | W gains the channel; R gains its readers if the agent can write |
| A member gives the agent a writing role. Any member who can write can make a guest a member. | Channel members | R gains the channel's readers |
| The agent joins an open channel as a member, or writes in it without joining. Both work in an open channel with `join_role = member`. | The agent's own key | R becomes the whole community |
| The agent creates a channel and adds people to it. The creator owns the channel, so it can add anyone. | The agent's own key | W gains the channel; R gains everyone the agent adds |
| The agent opens a DM or group DM. `handle_dm_open` has no agent check. | The agent's own key | W and R gain the DM's participants |
| Someone opens a DM or group DM that includes the agent, or adds the agent to one. Any DM member can add it, and a person can add it with the DM's history. Adding the agent, or opening a group DM with it, checks `channel_add_policy` ([#9125](https://github.com/block/buzz/pull/9125)), but that defaults to `anyone`. A one-to-one DM with the agent ignores the policy: anyone may message an agent. | Anyone who opens the DM, or any DM member | W and R gain the DM's participants |
| The agent joins the community, through an invite or a community add (kind 9030). The relay then joins it to every default channel, as a self-join. Default channels are open, so with `join_role = member` the agent can post in them. | Whoever admits the agent to the community | R becomes the whole community |
| The agent publishes a community-readable event outside a channel: profile (kind 0), text note (kind 1), long-form, user status, git events, lists, or a gift wrap to any recipient. | The agent's own key | R becomes everyone who can read that event |
| Someone joins a channel where the agent can post. New members can read the channel's history. | Whoever the channel's admission rules allow | R grows |
| An admin changes a channel's visibility from private to open. | Channel admins | R becomes the whole community |

Four of these paths use the agent's own key. The broker design takes the key away from the agent process for that reason. That is not necessary: the relay already knows who owns each agent (`agent_owner_pubkey`, which cannot change), so it can refuse those actions when the agent tries them.

## Design

The changes below hand each of those paths back to the owner, or turn it into a choice the owner makes knowingly.

### Disclosure policies

The rules in this design cost utility. A cleared agent cannot post a note, write in an open channel, set up a channel for a team, or change its profile freely. Many agents do all of these. So the rules apply only to agents with a clearance. An agent with no clearance works as it does now.

#### Policies, labels and clearances

A **policy** names one kind of information: customer data, personnel data, one investigation, one person's own information. It has an ordered list of levels, starting at *none*. Levels compare only within a policy.

A channel's **label** is the most sensitive level it may hold on each policy. A person's or agent's **clearance** is the most sensitive level they may read on each policy. Everything is at *none* unless someone sets it higher.

A policy with only *none* and *secret* is a **compartment**. It keeps apart audiences that would otherwise share a label, for example two investigations.

#### The rules

On every policy, for every action, whatever the membership:

1. **Read:** you can read a channel only if your clearance is at least its label.
2. **Write (agents only):** an agent can write in a channel only if its label is at least the agent's clearance.

So a cleared agent reads at or below its clearance and writes at or above it. DMs are unlabelled unless someone labels them, as private channels are. Everything outside channels is at *none*, so a cleared agent cannot write there, except for its profile with the owner's approval. A channel that a cleared agent creates gets the agent's clearance as its label.

When a label goes up or a clearance goes down, nobody is removed. The rules refuse what they no longer allow.

#### Cleared agents

For an agent with any clearance, two more rules:

- **Only the owner puts it in a channel.** It can join open channels by itself. Clearance does not let it join a labelled channel. Clearance says it may hold that kind of data. Membership says it may hold this channel's data: the channel's admins approve each member, as a data owner approves access to a dataset. If a cleared agent could join two channels with the same label, it could move one channel's content to the other's members, and the first channel's admins never approved that. This rule starts strict on purpose. It can be relaxed later, for example to let a cleared agent join any channel it is cleared for.
- **It adds only its owner.**

An agent with no clearance works as it does now. To protect a channel from it, label the channel.

#### Ratchets and authority

- Labels only go up.
- A labelled channel is private by definition, and stays private. Only a private channel can be labelled, and the relay refuses to make a labelled channel open, because that would give its content to everyone.
- An agent's clearance only goes up, because it remembers what it read.
- A clearance can expire. For an agent, expiry cuts its reads, and its write floor stays.
- Each policy's administrators set people's clearances. An owner can clear their agent up to the owner's own clearance.

#### System ratings

A label cannot be higher than the rating of the system that holds the data. Buzz has a rating, so no label can go above it. An agent's runtime is a system too, and so is its model provider. So an agent's clearance cannot be higher than its owner's clearance or its runtime's rating.

#### Labels are hidden

You see a label only if you are cleared for it. Everyone else gets "not authorized".

#### Possible: declassify

Content comes down only through a person. A cleared agent writes a draft in a channel where it can write. A person who can read that channel posts the draft in a lower channel as their own message, after any edits. Rule 2 does not apply to people, so the relay needs nothing new. Clients can offer "post this draft to…".

The usual way down is to reduce sensitivity: pseudonymize (replace direct identifiers), de-identify (remove direct and quasi-identifiers) or anonymize (make re-identification impossible). A transform can move content to another policy, not only to a lower level: de-identified personal data can become business data. The agent can do the transform in its draft. The person decides that it is done.

#### Agents with no clearance, and named cases

For information flow, a person is an agent with no clearance. A person reads the channels they are in, writes wherever they may, and anyone can add them to a channel or open a DM with them. The difference is who decides: a person decides with their own judgment, while an agent can be steered by anyone who can prompt it. The relay records an agent's owner only when the agent authenticates with its NIP-OA owner attestation or an agent token; a kind 0 attestation alone records nothing. An agent that never authenticates that way looks like a person to the relay. No clearance can apply to an agent until its owner is recorded.

Some cases have names:

- **Unrestricted** is an agent with no clearance.
- **Restricted** is a cleared agent.
- **Public** is an agent with no clearance kept out of private channels. It reads nothing private. Anyone can open a DM with it, so one person's DM can reach another person. Clients tell people to treat what they tell it as public. A public agent cannot stay in a channel that becomes private, so an open-to-private change is refused while a public agent is in the channel: remove the public agents first.
- **Personal** is an agent cleared on its owner's personal policy. It reads with the owner's authority, so the app gives the owner's read tools only to personal agents. Buzz cannot see what it sends outside Buzz, through other tools or through model inference on another member's node (see [What this doesn't stop](#what-this-doesnt-stop)).

One rule applies to every channel, whatever the agent's clearance: no private-to-open changes, except in an unlabelled channel that has only ever held its opener and the opener's agents (see [Membership is a decision about the channel](#membership-is-a-decision-about-the-channel)). It protects people as well as agents.

**Possible: a write-only role.** A member that can post in a channel but cannot read it. R grows and W doesn't. It fits integrations and notifiers that post into many private channels: they no longer read those channels, so content can't move between them through the key. The relay refuses the key's reads of that channel on every read path. A missed read path only lets W grow, as it does today.

### Only the owner puts an agent in a channel

To give the owner control of W, make the owner the only one who can change it. Only the agent's owner can add it to a channel, of any kind. Other members cannot.

The relay already has most of this. Each agent has a `channel_add_policy` (`anyone`, `owner_only` or `nobody`), and `owner_only` refuses adds by anyone but the owner. But the policy defaults to `anyone`, and the agent's own key sets it. For a cleared agent, the relay treats the policy as `owner_only` or stricter, and the agent can make it stricter but never looser.

The agent cannot join a private channel on its own. It can join an open channel by itself. That adds only public content to W, and rule 2 keeps it from writing there, so it adds nothing to R. It can create a channel, with limits (see [Channels the agent creates](#channels-the-agent-creates)).

DMs and group DMs are channels too, so the same rule applies: only the owner can open one that includes a cleared agent. A harness that forwards only its owner's messages to the model (for example `--respond-to owner-only`) does not close this path, because the agent can still read the DM with its own key. The relay has to refuse it.

Nothing private reaches the agent unless the owner put it there.

### Channels the agent creates

An agent sometimes needs a channel of its own, for example a system channel that holds its configuration. The relay makes the creator the channel's owner, so a channel that the agent creates is in W, and everyone it adds is in R. The relay limits this with the same rule as everywhere else:

- A channel that a cleared agent creates gets the agent's clearance as its label. A labelled channel is private, and stays private.
- The agent can add only its owner, and can change only its owner's role. Anyone else gets in only through people: its owner, or someone that the channel's admission rules allow.
- The agent can still lower roles and remove members, because that only makes R smaller.

This is the largest cost of clearance. A cleared agent cannot set up a working channel for a team and invite the team. Its owner has to add the people. A configuration channel is not affected: its roster is the agent and its owner.

### Huddles

A Huddle is a private channel: its room. Listening is reading and speaking is writing, so every rule above applies to the room as it does to any private channel or DM, on the audio relay as on the event relay.

- **Label:** a room gets its parent chat's label when it starts. Labels only go up.
- **Admission:** members of the parent chat join the room automatically. That is the room's admission rule, so the people in R include the parent chat's members and anyone added to the room. Every join, automatic or not, passes rule 1.
- **Agents:** an agent takes part only as a member of the room. Members can add agents, as in any private channel. A cleared agent joins only when its owner adds it, and only a room labelled at or above its clearance. The audio relay refuses its audio in any other room.
- **What the agent hears:** the relay sends the agent all of the room's audio. Its client chooses whose speech it acts on, but that is not enforcement, so W includes everything said in the room.

### Agents don't publish outside channels

An event outside a channel bypasses R entirely. A kind 1 note or a profile update is readable by whoever the relay lets read it, and no channel membership is involved. Everything outside channels is at *none*, so rule 2 refuses any event from a cleared agent that the relay would store outside a channel. An `h` tag does not count when the relay stores that kind community-wide anyway, as it does for kind 0, kind 1, long-form, user status, git events and lists.

The exceptions are kinds whose readers the relay already limits to the agent and its owner: the author-only kinds, and turn metrics (which only the `p`-tagged owner can read). One more exception does not limit its readers: the agent's profile (kind 0). Clients need it, because it carries the NIP-OA owner attestation that tells them the key is an agent, and only the agent's key can sign it. Its fields are public, so the profile is a leak path: the agent can write anything into them. Kind 0 is replaceable, so the agent can change it at any time, and each new version reaches everyone who reads it.

For a cleared agent, the relay accepts a kind 0 only when it carries the owner's signature over its content (a profile approval tag). The owner approves each profile change, and changes are rare. For an auditable agent, the publisher signs the profile from Git. Publish-once is not enough: the agent can read first and then put what it read into its one profile.

Refusing by default is what matters here. A new kind is covered without anyone having to remember that agents exist; it only gets through if someone deliberately adds it to the exceptions.

### Possible: step-up for actions with the owner's authority

Some work needs more than the agent's own access, for example summarizing a private channel that the agent is not in, or posting as the owner from a CLI. An app can do this with the owner's session or key. The relay then sees the owner, not the agent, so none of the rules above apply. This has happened without a decision by the owner: [buzz-app#2745](https://github.com/block/buzz-app/pull/2745) and [buzz-app#2881](https://github.com/block/buzz-app/pull/2881) gave every bundled agent read access to all of the owner's channels, DMs and Canvases, until [buzz-app#3123](https://github.com/block/buzz-app/pull/3123) removed those tools.

One possible answer is a step-up: an action with the owner's authority that the owner approves explicitly, for a stated scope and time, that is attributable to the agent, counted in W and R, and revocable. This is not part of this design. It widens what an agent can reach, so it would need careful boundaries and its own spec.

### Membership is a decision about the channel

By default, private channels and DMs trust their members, for agents as for people. A member who adds an agent chooses to share what they can read with the agent's R, as a member who adds a person shares the history with that person. Clients show each agent in a conversation and where it can post. The level rules still apply: a cleared agent cannot enter a channel or DM labelled lower than its clearance on any policy. A channel that needs more than member trust gets a label.

The owner cannot control who joins someone else's channel. So when the owner makes the agent a member of a channel, what the owner accepts is the channel's **admission rules**, not its current member list. R includes everyone those rules might let in later, and because new members can read history, those people also get everything the agent wrote before they arrived. Clients have to say this:

| Channel | Who can join R later | What the owner is accepting |
|---|---|---|
| Open | Anyone in the community | R is everyone. |
| Private | Anyone a member adds | Trust in every current member's judgment |

Accepting the rules only means something if the rules stay put. One relay change makes sure they do:

- **Refuse private-to-open visibility changes, with one exception.** Making a private channel open changes the admission rules out from under everyone who joined under the old ones, people included. That is a good rule with or without agents. An admin who wants an open channel can create one.
  The exception is an unlabelled channel where no other person was ever a member. Every current and former member must be that person or one of that person's agents. A cleared agent writes only in labelled channels, which never open, so no cleared agent's writes are in such a channel. Opening it, for example a Solo chat, is a one-time disclosure by a person of what that person can already read. After it opens, it is an ordinary open channel, and rule 2 keeps the owner's cleared agents from writing there.

**Later improvement: limit adds to admins.** Every private channel has the same admission rule: any member can add anyone. So making the agent a member of a private channel means trusting every current member's judgment. A later channel setting can limit adds to owners and admins. With it, an owner can choose a private channel where only the admins can grow R, and what the owner accepts becomes trust in the channel's admins. This is not needed for safety, because the owner accepts whatever the admission rules are. It gives the owner a tighter R to choose. The same setting helps announce channels, where it keeps the set of writers under the admins' control.

The owner gets an alert when someone joins a channel where the agent is a member. That is not what keeps the data safe, since the owner accepted the rules up front. It lets the owner notice a channel heading somewhere unexpected, and demote or remove the agent.

### W remembers

Removing an agent from a channel stops new information from coming in, but the agent still remembers what it already read. So W keeps every channel the agent has ever read, with how far it read: channel X, up to the time it left. A channel the agent is still in is in W up to now. The relay does not keep this yet: it keeps one membership row per channel and key, and a re-add clears the last leave. W needs membership history that the relay does not store today.

### When the owner is not a person

The relay checks `agent_owner_pubkey`, and the owner is whichever key signed the agent's NIP-OA attestation. It need not be a person. An auditable agent uses this: its owner is its publisher, a CI key that signs only reviewed changes from Git.

The rules above do not change. Only the owner adds the agent to a channel or raises its role, and here the owner is the publisher. So every change to W and R is a reviewed change in Git. The publisher holds no private information of its own in Buzz, so the agent can read only what those reviewed changes gave it.

Its clearance comes from Git too. An auditable agent is normally cleared, so that every path into R is a reviewed change.

One thing assumes that the owner is a person, and it moves: the join alert goes to the people who answer for the agent, the on-call group named in the auditable runtime's deployment configuration.

### Product work

1. **Set labels and clearances.** Channel admins label channels. Policy administrators set people's clearances. The owner clears an agent up to the owner's own clearance.
2. **Show a label only to people cleared for it.** Everyone else gets "not authorized".
3. **Ask the owner to approve profile changes** of a cleared agent.
4. **Offer the add of a cleared agent only to its owner.** Clients that let any member add one directly should not.
5. **Show each agent in a channel or DM, and where it can post.**

## What this doesn't stop

- **Flows the owner accepts.** Say the owner makes the agent a member of an Alice-and-Bob channel and an Alice-and-Carol channel. Bob's information can now reach Carol. The owner chose both memberships, and nothing stops that choice. The broker design would block it.
- **People the channel's rules let in.** Admins or members can add people to R, and those people can read history. That is what the owner accepted when making the agent a member.
- **Actions with the owner's authority.** An app acting with the owner's session or key is not an agent action, so these rules do not see it.
- **The owner's information outside Buzz.** Files, documents, chat systems and whatever other tools the agent uses. The broker design leaves this out too.

## Alternatives considered

| Option | Why not |
|---|---|
| Do nothing | Any agent in a private channel can leak it to a public one, and a prompt-injected agent can widen its own audience. |
| Broker design (audience-bound instances, keyless agents, labeled memory) | It enforces a stronger rule: no flow between different private audiences, even inside one agent. But it needs a trusted broker, key custody outside the agent process, per-audience instances and labeled managed memory. This design uses the access control Buzz already has, and puts the decision in front of the owner. |
| Label each agent event with its sources and check every read | It handles late joins without trusting admission rules, but it is event-level IFC: a second access-control system alongside channels, which every read path has to apply. This design keeps channels as the only authority. |
| Owner controls only non-public memberships | Not enough on its own, because the first open channel where the agent can write makes R everyone. Rule 2 closes that: open channels are at *none*, so a cleared agent cannot write in one. |

## Risks

- **Owner decisions.** This is only as good as the owner's decisions about which channels the agent is in, and in which role.
- **A wrong exception.** The exception list is the only way an agent can publish outside a channel, so every kind on it, except the profile, has to really limit its readers to the agent and its owner.
- **The agent's profile is a leak path** for an agent with no clearance. Its fields are public and the agent can change them at any time.
- **Owners leave agents uncleared for convenience.** Then only labels protect a channel from them. To protect a channel from an agent with no clearance, label the channel.
- **Refusing private-to-open breaks a workflow.** Some admins start a channel private and open it later. Unless the channel holds only them and their agents, they have to create a new open channel instead.
- **Opening a Solo chat publishes what the owner's uncleared agents wrote there**, including what they drew from their other channels. Accepted: a person discloses what they can read.

## Rollout

1. ✅ Announce channels: guests are read-only everywhere ([#8143](https://github.com/block/buzz/pull/8143)).
2. Refuse private-to-open visibility changes ([#9307](https://github.com/block/buzz/pull/9307)), except in an unlabelled channel where every current and former member is the person opening it or one of that person's agents.
3. Add policies, labels and clearances, all at *none*.
4. Enforce rule 1 and hidden labels.
5. Enforce rule 2 and the cleared-agent rules: only the owner adds a cleared agent to a channel or opens a DM that includes it, the agent adds only its owner, and a channel it creates gets its clearance as its label. The audio relay enforces the same rules for Huddle rooms.
6. For cleared agents: refuse events that the relay would store outside a channel, after checking what agents publish now, and require profile approval.

Each step only refuses actions, so backing one out means turning the rule off. Existing agents keep their memberships and roles; the rules apply to new actions.

The wire formats for labels, clearances and the profile approval tag are specified when they are built, in the NIPs that own agent identity.

## Proposal: terminology

Not applied yet. These names follow common data-classification and multilevel-security usage.

A domain has levels. Channels, people and agents are labeled for a domain at one of its levels, or unlabeled for that domain. Unlabeled is below every level. A channel, person or agent with any label is labeled; one with none is unlabeled.

| Now | Proposed |
|---|---|
| Disclosure policies (section) | Domains and labels |
| Policy | Domain |
| Policy administrators | Domain administrators |
| Levels | Levels (no change) |
| *none* | Unlabeled for that domain |
| Channel's label | Channel's label (no change) |
| Person's or agent's clearance | Person's or agent's label |
| Cleared agent, Restricted | Labeled agent |
| Agent with no clearance, Unrestricted | Unlabeled agent |
| Cleared-agent rules | Labeled-agent rules |
| Compartment | Domain with one level |
| Personal policy | Personal domain |
| An owner clears their agent up to the owner's own clearance | An owner labels their agent up to the owner's own label |
| Clearance only goes up | An agent's label only goes up |
| System rating | Accreditation |
| Rule 1: clearance at least the label | Rule 1: the reader's label is at least the channel's label |
| Rule 2: label at least the agent's clearance | Rule 2: the channel's label is at least the agent's label |

No change: Public, Personal, declassify, hidden labels, W and R.

With one label, an agent cannot read less and still write as high as before. So when an agent's label expires, the label stays in place and the agent is removed from labeled channels.
