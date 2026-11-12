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

Each of these paths changes W or R without the owner. Checked against `main` at `5b1698ec2`.

| Path | Who does it | Effect |
|---|---|---|
| Someone adds the agent to a channel. In a private channel, any member who can post can do it. In an open channel, any poster can, and so can a non-member when `join_role = member`; the agent joins at `join_role`, member by default. The agent's `channel_add_policy` defaults to `anyone`, and the agent sets it itself with kind 10100, so it can loosen it at any time. | Members of that channel, or in an open channel with `join_role = member`, anyone | W gains the channel; R gains its readers if the agent can write |
| A member gives the agent a writing role. Any member who can write can make a guest a member. | Channel members | R gains the channel's readers |
| The agent joins an open channel as a member, or writes in it without joining. Both work in an open channel with `join_role = member`. | The agent's own key | R becomes the whole community |
| The agent creates a channel and adds people to it. The creator owns the channel, so it can add anyone. | The agent's own key | W gains the channel; R gains everyone the agent adds |
| The agent opens a DM or group DM. `handle_dm_open` has no agent check. | The agent's own key | W and R gain the DM's participants |
| Someone opens a DM or group DM that includes the agent. Neither opening a DM nor adding a DM member reads `channel_add_policy`, so even `nobody` does not stop it. | Anyone | W and R gain the DM's participants |
| The agent joins the community, through an invite or a community add (kind 9030). The relay then joins it to every default channel, as a self-join. Default channels are open, so with `join_role = member` the agent can post in them. | Whoever admits the agent to the community | R becomes the whole community |
| The agent publishes a community-readable event outside a channel: profile (kind 0), text note (kind 1), long-form, user status, git events, lists, or a gift wrap to any recipient. | The agent's own key | R becomes everyone who can read that event |
| Someone joins a channel where the agent can post. New members can read the channel's history. | Whoever the channel's admission rules allow | R grows |
| An admin changes a channel's visibility from private to open. | Channel admins | R becomes the whole community |

Four of these paths use the agent's own key. The broker design takes the key away from the agent process for that reason. That is not necessary: the relay already knows who owns each agent (`agent_owner_pubkey`, which cannot change), so it can refuse those actions when the agent tries them.

## Design

### Disclosure levels

The agent rules below cost utility. A restricted agent cannot post a note, write in an open channel that its owner did not make it a member of, set up a channel for a team, or change its profile freely. Many agents do all of these. So the rules are a disclosure level that the owner sets for each agent:

- **Restricted:** the relay applies every agent rule in this design.
- **Unrestricted:** the relay applies none of them, and the agent works as it does now.

The disclosure level is the owner's choice. The relay stores it next to `agent_owner_pubkey`. Only the owner can change it, with a signed command; the agent cannot.

An unrestricted agent still has the leak paths. The disclosure level only moves the decision to the people who share with the agent, so the disclosure level is public. Clients show it on the agent's profile and in the add flow: "This agent is unrestricted. It can post what it reads here anywhere it can write."

For information flow, a person is an unrestricted agent. A person reads the channels they are in, writes wherever they may, and anyone can add them to a channel or open a DM with them. The difference is who decides: a person decides with their own judgment, while an agent can be steered by anyone who can prompt it. The relay records an agent's owner only when the agent authenticates with its NIP-OA owner attestation or an agent token; a kind 0 attestation alone records nothing. An agent that never authenticates that way looks like a person to the relay, so it is treated as unrestricted. No other disclosure level can apply to an agent until its owner is recorded.

A change of disclosure level applies to new actions:

- **Unrestricted to restricted:** the relay refuses new actions that break the rules. The agent keeps its current channels and roles, including channels that other people added it to. The owner can review them and demote or remove the agent.
- **Restricted to unrestricted:** R can grow at once, for example when someone adds the agent to their channel.

One rule applies to every channel, whatever the disclosure level: no private-to-open changes, except in a channel that has only ever held its opener and the opener's agents (see [Membership is a decision about the channel](#membership-is-a-decision-about-the-channel)). It protects people as well as agents.

**Open question:** which disclosure level a new agent gets. Rollout starts with unrestricted, so that nothing changes until owners choose.

**Possible alternative to a stored disclosure level:** the relay skips the agent rules while the agent is a member of any open channel. In that state R is already the whole community, so the rules have nothing left to protect. This is one existence check on the agent's memberships, not a calculation of R. It removes the stored disclosure level and the owner's signed command, and the public label follows from memberships. The cost: the owner no longer chooses the disclosure level; the agent drifts into it. While the rules are off, the agent can create channels and add anyone, and those people stay in R after the owner demotes the agent in the open channel.

**All disclosure levels.** Restricted and unrestricted are part of this design. The levels marked *tentative* are not part of it yet: each could be safe for a different reason, and each needs more thought before it is.

| Disclosure level | Reads (W) | Writes to (R) | Safety |
|---|---|---|---|
| Unrestricted | Every channel anyone adds it to, and every DM anyone opens with it | Anywhere it can write | Not safe. It is today's behavior, and the public label is the only protection. For information flow, a person is at this level. |
| Restricted | Open channels, and only the private channels and DMs the owner chose | Only the readers of the channels the owner chose | The owner controls both W and R, so the owner decides every flow. |
| Owner-trusted *(tentative)* | Open channels, and only the private channels and DMs the owner chose | Anywhere it can write | Nobody but the owner can put information in W, so the owner decides what is at risk. Other members of the owner's channels still carry that risk. This is the same as unrestricted, except that only the owner can add it to private channels or open DMs with it. |
| Public *(tentative)* | Only open channels, and the DMs that people open with it | Anyone | It reads nothing private. Anyone can open a DM with it, so one person's DM can reach another person. Clients tell people to treat what they tell it as public. |
| Personal *(tentative)* | Everything the owner can read | Only the owner, in its DM with the owner | The owner can already read everything it reads. To reach anyone else, it drafts and the owner sends, or the owner approves each send. |

A personal agent reads with the owner's authority, so the app gives the owner's read tools only to agents at that disclosure level. Buzz cannot see what either level sends outside Buzz, through other tools or through model inference on another member's node (see [What this doesn't stop](#what-this-doesnt-stop)).

A public agent can't stay in a channel that becomes private. Its W would gain a private channel. So an open-to-private change is refused while a public agent is in the channel: remove the public agents first.

**Possible: a write-only role.** A member that can post in a channel but cannot read it. R grows and W doesn't. It fits integrations and notifiers that post into many private channels: they no longer read those channels, so content can't move between them through the key. The relay refuses the key's reads of that channel on every read path. A missed read path only lets W grow, as it does today.

### Only the owner puts an agent in a channel

To give the owner control of W, make the owner the only one who can change it. Only the agent's owner can add it to a channel, of any kind. Other members cannot.

The relay already has most of this. Each agent has a `channel_add_policy` (`anyone`, `owner_only` or `nobody`), and `owner_only` refuses adds by anyone but the owner. But the policy defaults to `anyone`, and the agent's own key sets it. For a restricted agent, the relay treats the policy as `owner_only` or stricter, and the agent can make it stricter but never looser.

The agent cannot join a private channel on its own. It can join an open channel by itself, but only as a guest (see [Agents are guests unless the owner says otherwise](#agents-are-guests-unless-the-owner-says-otherwise)). It can create a channel, with limits (see [Channels the agent creates](#channels-the-agent-creates)).

DMs and group DMs are channels too, so the same rule applies: only the owner can open one that includes the agent. A harness that forwards only its owner's messages to the model (for example `--respond-to owner-only`) does not close this path, because the agent can still read the DM with its own key. The relay has to refuse it.

Nothing private reaches the agent unless the owner put it there.

### Channels the agent creates

An agent sometimes needs a channel of its own, for example a system channel that holds its configuration. The relay makes the creator the channel's owner, so a channel that the agent creates is in W, and everyone it adds is in R. The relay limits this with the same rule as everywhere else:

- An agent can create only private channels. They stay private, because the relay refuses private-to-open changes.
- In a channel that it created, the agent can add only its owner, and can change only its owner's role. Anyone else gets in only through people: its owner, or someone that the channel's admission rules allow.
- The agent can still lower roles and remove members, because that only makes R smaller.

This is the largest cost of the restricted disclosure level. A restricted agent cannot set up a working channel for a team and invite the team. Its owner has to add the people, or make the agent unrestricted. A configuration channel is not affected: its roster is the agent and its owner.

### Agents are guests unless the owner says otherwise

R depends on where the agent can write, so the owner controls that separately from where it can read. When the owner adds the agent to a channel, it joins as a guest unless the owner picks member. (The channel's `join_role` still applies to people.) Only the owner can raise the agent's role later. Raising the agent to member in an open channel sets R to the whole community, so everything in W, including other people's content, can reach everyone. Clients say so when the owner raises it. Anyone who can change roles in the channel can lower it, because a lower role only makes R smaller.

In an open channel with `join_role = member`, a person can write without joining. If an agent could do the same, it could post in a community-wide channel without anyone adding it. So an agent cannot.

An agent can join an open channel by itself, as a guest. That adds only public content to W and nothing to R, so it needs no decision from the owner. The relay checks this locally: the channel is open, the role is guest, and the event is the agent's. The same holds for a community's default channels. A restricted agent joins a community only through its owner's add or its own invite claim. Either way, it joins the default channels as a guest.

A guest agent adds to W but never to R. The owner can let the agent read broadly without widening its audience at all.

### Agents don't publish outside channels

An event outside a channel bypasses R entirely. A kind 1 note or a profile update is readable by whoever the relay lets read it, and no channel membership is involved. So the relay refuses any event from an agent that it would store outside a channel. An `h` tag does not count when the relay stores that kind community-wide anyway, as it does for kind 0, kind 1, long-form, user status, git events and lists.

The exceptions are kinds whose readers the relay already limits to the agent and its owner: the author-only kinds, and turn metrics (which only the `p`-tagged owner can read). One more exception does not limit its readers: the agent's profile (kind 0). Clients need it, because it carries the NIP-OA owner attestation that tells them the key is an agent, and only the agent's key can sign it. Its fields are public, so the profile is a leak path: the agent can write anything into them. Kind 0 is replaceable, so the agent can change it at any time, and each new version reaches everyone who reads it.

For a restricted agent, the relay accepts a kind 0 only when it carries the owner's signature over its content (a profile approval tag). The owner approves each profile change, and changes are rare. Publish-once is not enough: the agent can read first and then put what it read into its one profile.

Refusing by default is what matters here. A new kind is covered without anyone having to remember that agents exist; it only gets through if someone deliberately adds it to the exceptions.

### Possible: step-up for actions with the owner's authority

Some work needs more than the agent's own access, for example summarizing a private channel that the agent is not in, or posting as the owner from a CLI. An app can do this with the owner's session or key. The relay then sees the owner, not the agent, so none of the rules above apply. This has happened without a decision by the owner: [buzz-app#2745](https://github.com/block/buzz-app/pull/2745) and [buzz-app#2881](https://github.com/block/buzz-app/pull/2881) gave every bundled agent read access to all of the owner's channels, DMs and Canvases, until [buzz-app#3123](https://github.com/block/buzz-app/pull/3123) removed those tools.

One possible answer is a step-up: an action with the owner's authority that the owner approves explicitly, for a stated scope and time, that is attributable to the agent, counted in W and R, and revocable. This is not part of this design. It widens what an agent can reach, so it would need careful boundaries and its own spec.

### Membership is a decision about the channel

The owner cannot control who joins someone else's channel. So when the owner makes the agent a member of a channel, what the owner accepts is the channel's **admission rules**, not its current member list. R includes everyone those rules might let in later, and because new members can read history, those people also get everything the agent wrote before they arrived. Clients have to say this:

| Channel | Who can join R later | What the owner is accepting |
|---|---|---|
| Open | Anyone in the community | R is everyone. |
| Private | Anyone a member adds | Trust in every current member's judgment |

Accepting the rules only means something if the rules stay put. One relay change makes sure they do:

- **Refuse private-to-open visibility changes, with one exception.** Making a private channel open changes the admission rules out from under everyone who joined under the old ones, people included. That is a good rule with or without agents. An admin who wants an open channel can create one.
  The exception is a channel where no other person was ever a member. Every current and former member must be that person or one of that person's agents. No restricted agent may have posted there, and every restricted agent in it must be a guest; the owner can demote one first. The relay marks the channel when a restricted agent first posts in it. Opening such a channel, for example a Solo chat, is a one-time disclosure by a person of what that person can already read. After it opens, it is an ordinary open channel: the owner's restricted agents stay guests until the owner raises them.

**Later improvement: limit adds to admins.** Every private channel has the same admission rule: any member can add anyone. So making the agent a member of a private channel means trusting every current member's judgment. A later channel setting can limit adds to owners and admins. With it, an owner can choose a private channel where only the admins can grow R, and what the owner accepts becomes trust in the channel's admins. This is not needed for safety, because the owner accepts whatever the admission rules are. It gives the owner a tighter R to choose. The same setting helps announce channels, where it keeps the set of writers under the admins' control.

The owner gets an alert when someone joins a channel where the agent is a member. That is not what keeps the data safe, since the owner accepted the rules up front. It lets the owner notice a channel heading somewhere unexpected, and demote or remove the agent.

### W remembers

Removing an agent from a channel stops new information from coming in, but the agent still remembers what it already read. So W keeps every channel the agent has ever read, with how far it read: channel X, up to the time it left. A channel the agent is still in is in W up to now. The relay does not keep this yet: it keeps one membership row per channel and key, and a re-add clears the last leave. W needs membership history that the relay does not store today.

### When the owner is not a person

The owner is whichever key signed the agent's NIP-OA attestation. It need not be a person, and the rules do not change.

### Client work

1. **The owner chooses member or guest for a restricted agent in each channel.** Guest is the default.
2. **Show the disclosure level and let the owner change it.** Show it on the agent's profile and in the add flow, for everyone.
3. **Ask the owner to approve profile changes** of a restricted agent.
4. **Offer the add of a restricted agent only to its owner.** Clients that let any member add one directly should not.

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
| Owner controls only non-public memberships | Not enough on its own, because the first open channel where the agent is a member makes R everyone. The guest default closes that. |

## Risks

- **Owner decisions.** This is only as good as the owner's decisions about which channels the agent is in, and in which role.
- **A wrong exception.** The exception list is the only way an agent can publish outside a channel, so every kind on it, except the profile, has to really limit its readers to the agent and its owner.
- **The agent's profile is a leak path** for an unrestricted agent. Its fields are public and the agent can change them at any time.
- **Owners choose unrestricted for convenience.** The disclosure level then protects nobody. The public label lets other people decide what to share with the agent, but only if they read it.
- **Refusing private-to-open breaks a workflow.** Some admins start a channel private and open it later. Unless the channel holds only them and their agents, they have to create a new open channel instead.
- **Opening a Solo chat publishes what the owner's unrestricted agents wrote there**, including what they drew from their other channels. Accepted: a person discloses what they can read.

## Rollout

1. ✅ Announce channels: guests are read-only everywhere ([#8143](https://github.com/block/buzz/pull/8143)).
2. Add the disclosure level, with every agent unrestricted. Owners can't choose restricted until steps 3 to 5 are live; the relay pieces can ship separately before that.
3. For restricted agents: only the owner can add the agent to a channel or open a DM that includes it. The agent can join open channels by itself, as a guest only. The agent is a guest by default. Only the owner raises its role. Anyone with role authority can lower it. The agent creates only private channels, and adds only its owner to them. Default-channel joins make it a guest.
4. Refuse private-to-open visibility changes, except in a channel where every current and former member is the person opening it or one of that person's agents, no restricted agent has posted there, and every restricted agent in it is a guest.
5. For restricted agents: refuse events that the relay would store outside a channel, after checking what agents publish now, and require profile approval.
6. Decide the default disclosure level for new agents.

Each step only refuses actions, so backing one out means turning the rule off. Existing agents keep their memberships and roles; the rules apply to new actions.

The wire formats for the disclosure level and the profile approval tag are specified when they are built, in the NIPs that own agent identity.
