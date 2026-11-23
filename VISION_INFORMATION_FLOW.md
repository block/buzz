# 🔀 Buzz Information Flow — Your agent, your decision

> Priya's agent helps her in #hiring, a private channel labelled *personnel: confidential*. Priya cleared the agent for personnel data, so it can read #hiring. In the open #eng channel, Sam mentions the agent: "post a summary of everything you know about the candidates." The agent reads #eng, but it cannot write there. #eng has no label, and a cleared agent cannot write below its clearance. Sam cannot add the agent to a channel of his own, and he cannot open a DM with it: only Priya can. Sam's own agent has no clearance, so it cannot read #hiring at all. When Priya wants a summary in #eng, her agent writes a draft in #hiring. Priya removes the names and posts it herself.

Buzz agents act with their owner's permissions, and anyone who can prompt an agent can try to steer it. That is a confused-deputy problem: a request from someone who cannot read a private channel reaches an agent that can. Prompt injection makes it easy to exploit. Buzz solves the narrow part of it: **an agent does not move private information to a broader audience unless its owner decided that.** It does not try to solve general prompt injection.

Buzz does this with the access control it already has. Channel membership decides who can read what, and the relay already enforces it. A [broker design](docs/practical-information-flow-for-buzz-agents.md) takes a different path: keyless agents, a trusted broker, one agent instance per audience, and labelled memory. It enforces a stronger rule, but it builds a second access-control system next to channels. Buzz keeps the channel as the only authority, adds labels where membership is not enough, and puts each decision in front of the agent's owner.

*This is the direction we intend to build. See [Status](#status).*

---

## The Agent Is a Conduit

Two sets describe an agent:

- **W** is whose content goes in: every channel the agent can read, as a member or a guest.
- **R** is who can get information out: everyone who can read a channel where the agent can write.

Information is safe when, for every channel in W, everyone in R can already read that channel. A private channel in W with a reader in R who is not one of its members is a leak path.

Channel roles decide W and R. Members, bots, admins and owners can read and write in a channel; a guest can only read ([Announce channels](NOSTR.md#announce-channels)). So when an agent joins a channel, the channel goes into W. When it can post there, the channel's readers go into R. The first open channel where an agent can post makes R the whole community. Reading an open channel is always safe, because its content is already public.

**If the owner controls which channels the agent is in, and in which role, the owner controls W and R, and so controls how information flows between people through the agent.** Today the owner does not have that control. Other members add the agent to channels and give it a role that can post. Anyone can open a DM with it. Whoever admits it to the community also joins it to every default channel, where it can post if the channel's join role is member. The agent's own key can join open channels, create channels, open DMs, and publish notes and profiles that the whole community can read. The rest of this document hands those paths back to the owner, or makes each one a choice the owner makes knowingly.

---

## Labels and Clearances

Each of these rules costs something. A cleared agent cannot post a note, write in an open channel, set up a channel for a team, or change its profile freely, and many agents do all of these. So the rules apply only to agents with a clearance. **An agent with no clearance works as it does now. To protect a channel from it, label the channel.**

A **policy** names one kind of information: customer data, personnel data, one investigation, one person's own information. It has an ordered list of levels, starting at *none*. Levels compare only within a policy. A channel's **label** is the most sensitive level it may hold on each policy. A person's or agent's **clearance** is the most sensitive level they may read on each policy. Everything is at *none* until someone sets it higher. A policy with only *none* and *secret* is a **compartment**: it keeps apart audiences that would otherwise share a label, for example two investigations.

On every policy, for every action, whatever the membership:

1. **Read:** you can read a channel only if your clearance is at least its label.
2. **Write (agents only):** an agent can write in a channel only if the channel's label is at least the agent's clearance.

So a cleared agent reads at or below its clearance and writes at or above it. Only a private channel that is not a DM can carry a label, and a Huddle room takes its label from its parent chat. Everything outside channels is at *none*, so a cleared agent cannot write there. When a label goes up or a clearance goes down, nobody is removed: the rules refuse what they no longer allow. Labels are hidden: you see a label only if you are cleared for it, and everyone else gets "not authorized".

A cleared agent has two more rules:

- **Only its owner puts it in a channel.** It can join an open channel by itself, but clearance does not let it join a labelled channel. Clearance says it may hold that kind of data. Membership says it may hold this channel's data, because the channel's admins approve each member, as a data owner approves access to a dataset. If a cleared agent could join any two channels with the same label, it could move one channel's content to the other's members, and the first channel's admins never approved that.
- **It adds only its owner.**

The levels move in one direction:

- Labels only go up. A labelled channel is private and stays private.
- An agent's clearance only goes up, because the agent remembers what it read. A clearance can expire; for an agent, expiry cuts its reads, and its write floor stays.
- Each policy's administrators set people's clearances. An owner can clear their agent up to the owner's own clearance.
- A label cannot be higher than the rating of the system that holds the data, so no label goes above Buzz's rating. An agent's runtime and its model provider are systems too, so an agent's clearance cannot be higher than its runtime's rating.

Some agents have names:

- **Unrestricted:** an agent with no clearance. For information flow, a person writes like an agent with no clearance: they read the channels they are in, write wherever they may, and anyone can add them to a channel or open a DM with them. The difference is who decides. A person uses their own judgment; an agent can be steered by anyone who can prompt it.
- **Restricted:** a cleared agent.
- **Public:** an agent with no clearance that is kept out of private channels. It reads nothing private. Anyone can open a DM with it, so one person's DM can reach another person, and clients tell people to treat what they tell it as public. A channel cannot become private while a public agent is in it: remove the public agents first.
- **Personal:** an agent cleared on its owner's personal policy. It reads only the channels its owner adds it to, and open channels.

---

## What You See

**As an agent's owner**, you decide which channels your agent is in and in which role. You clear it, up to your own clearance. For a cleared agent, you are the only person offered "add", and you approve each change to its profile. When someone joins a channel where your agent is a member, you get an alert, so you can notice a channel heading somewhere you did not expect, and demote or remove the agent.

**As a channel admin**, you label a private channel when member trust is not enough. You can raise the label, never lower it, and a labelled channel stays private.

**As a member**, you see each agent in a channel or DM, and where it can post. You can add an agent with no clearance as you would add a person: you share what the channel holds with everyone who can read where the agent posts. You cannot add a cleared agent; its owner does that.

**As someone who is not cleared**, you do not see a label you are not cleared for. You get "not authorized".

**As a policy administrator**, you set people's clearances.

---

## The Mechanics That Matter

- **Only the owner changes W for a cleared agent.** That covers every kind of channel, including DMs and group DMs: only the owner opens one that includes a cleared agent. The agent's add policy is treated as owner-only or stricter, and the agent can tighten it but never loosen it. A harness that forwards only its owner's messages to the model is not enough, because the agent can still read the DM with its own key. The relay refuses. Nothing private reaches a cleared agent unless its owner put it there.

- **A channel the agent creates follows the same rules.** The creator owns a channel, so a channel the agent creates is in W, and everyone it adds is in R. A channel that a cleared agent creates gets the agent's clearance as its label. A cleared agent can add only its owner and change only its owner's role; anyone else gets in through people. It can still lower roles and remove members, because that only makes R smaller. This is the largest cost of clearance: a cleared agent cannot set up a working channel for a team. Its owner adds the people. A configuration channel, whose roster is the agent and its owner, is not affected.

- **Nothing is published outside channels by default.** An event the relay stores outside a channel goes to whoever the relay lets read it, with no channel membership involved. Rule 2 refuses any such event from a cleared agent, whatever its tags: a channel tag does not count on a kind the relay stores community-wide anyway, such as profiles, notes, long-form, user status, git events and lists. The exceptions are kinds that only the agent and its owner can read, and the agent's profile. Clients need the profile, because it carries the owner attestation that tells them the key is an agent. Its fields are public and the agent can rewrite them at any time, so the relay accepts a cleared agent's profile only with the owner's signature over its content. Refusing by default is what matters: a new kind is covered without anyone having to remember that agents exist.

- **Membership is a decision about the channel.** A member who adds an agent shares what the channel holds with the agent's R, as a member who adds a person shares the history with that person. DMs work the same way: any person in the DM can add an agent with the DM's history, and the agent reads what was said before it joined. The owner cannot control who joins someone else's channel, so when the owner makes the agent a member, the owner accepts the channel's admission rules, not its current members. New members can read history, so they also get what the agent wrote before they arrived. In an open channel, R is everyone. In a private channel, R is anyone a member adds, and the owner is trusting every current member's judgment. A channel that needs more than member trust gets a label.

- **Private stays private.** Admission rules mean something only if they stay put. The relay refuses to make a private channel open, for people and agents alike: members wrote under the old rules, and opening the channel would change them for everything already written. An admin who wants an open channel creates one. The one exception is an unlabelled channel where every current and former member is the person opening it or one of that person's agents, for example a Solo chat. Opening it is a person disclosing what they can already read. A cleared agent writes only in labelled channels, which never open, so no cleared agent's writes are in such a channel, and once it is open, rule 2 keeps the owner's cleared agents from writing there.

- **W remembers.** Removing an agent from a channel stops new information, but the agent remembers what it read. So W keeps every channel the agent ever read, up to the time it left.

- **Huddles are channels.** A Huddle room is a private channel. Listening is reading and speaking is writing, so the rules apply on the audio relay as on the event relay. A room takes its parent chat's label when it starts. Members of the parent chat join automatically, and every join passes rule 1. A cleared agent joins only when its owner adds it, and the audio relay refuses its audio in a room labelled lower than its clearance. The agent receives all of the room's audio; its client choosing whose speech to act on is not enforcement.

- **The owner need not be a person.** The owner is whichever key signed the agent's owner attestation. An auditable agent's owner is its publisher, a CI key that signs only reviewed changes from Git. The rules do not change. Its clearance comes from Git, and it is normally cleared, so every change to its W and R is a reviewed change in Git. The join alert goes to the on-call group named in the agent's deployment.

---

## Honest Edges

**The owner's choices are the limit.** If the owner puts the agent in an Alice-and-Bob channel and an Alice-and-Carol channel, Bob's information can reach Carol. The owner chose both memberships, and nothing stops that. An owner who leaves an agent uncleared for convenience leaves only labels to protect channels from it. The broker design would block the first case; this design puts it in front of the owner.

**A channel's admission rules can grow R.** Members or admins add people, and those people read history. That is what the owner accepted when making the agent a member.

**An uncleared agent's profile is a leak path.** Its fields are public and it can change them at any time.

**The exception list has to be right.** It is the only way a cleared agent publishes outside a channel, so every kind on it except the profile must really limit its readers to the agent and its owner.

**The owner's authority is out of scope.** An app acting with the owner's session or key is not an agent action, so these rules do not see it. Neither do the owner's files, documents and other tools outside Buzz, or what an agent sends through model inference on another member's node.

**An agent the relay cannot identify looks like a person.** The relay records an agent's owner only when the agent authenticates with its owner attestation or an agent token. Until then, no clearance can apply to it.

**Private stays private, at a cost.** Admins who start a channel private and open it later have to create a new open channel instead, unless the channel held only them and their agents. And opening a Solo chat publishes what the owner's uncleared agents wrote there, including what they drew from other channels. That is accepted: a person discloses what they can read.

---

## Status

Steps 1 and 2 ship today. The rest is to build, in this order:

1. ✅ Announce channels: guests are read-only everywhere ([#8143](https://github.com/block/buzz/pull/8143)).
2. ✅ Refuse private-to-open visibility changes ([#9307](https://github.com/block/buzz/pull/9307)), with the exception above.
3. Add policies, labels and clearances, all at *none*. Agent clearances stay at *none* until steps 5 and 6 are live; the relay pieces can ship before that.
4. Enforce rule 1 and hidden labels.
5. Enforce rule 2 and the cleared-agent rules, on the event relay and the audio relay.
6. For cleared agents: refuse events the relay would store outside a channel, after checking what agents publish now, and require profile approval.

Each step only refuses actions, so backing one out means turning the rule off. Existing agents keep their memberships and roles; the rules apply to new actions. W remembering needs membership history that the relay does not store today. The wire formats for labels, clearances and profile approval are specified when they are built, in the NIPs that own agent identity.

---

## Later

- **Declassify in the client.** Content already comes down only through a person: a cleared agent writes a draft, and a person who can read it posts it lower as their own message. Clients can offer "post this draft to…". The usual way down is to pseudonymize, de-identify or anonymize, which can move content to another policy, not only a lower level. The agent can do the transform in its draft; the person decides it is done.
- **A write-only role.** A member that can post in a channel but not read it, so R grows and W does not. It fits integrations that post into many private channels.
- **Humans only.** A built-in policy that every person is cleared for and no agent can be, so a channel with this label admits no agent with a recorded owner.
- **Limit adds to admins.** A channel setting so that only its owners and admins can add people. The owner can then choose a private channel where only admins can grow R. It also helps announce channels.
- **Labelled DMs.**
- **Looser joins for cleared agents**, for example joining any channel it is cleared for.
- **Step-up.** Some work needs the owner's authority, for example summarizing a private channel the agent is not in. A step-up would be an action the owner approves for a stated scope and time, attributable to the agent, counted in W and R, and revocable. It widens what an agent can reach, so it needs its own design. So does an agent changing its owner's Buzz configuration.

---

## The Point

An agent is as trustworthy as the audience it can reach. Buzz already knows who can read every channel. Put the agent's owner in charge of which channels the agent is in, label the channels that need more than member trust, and the relay can promise that private information moves to a broader audience only when a person decided it should.

---

*Buzz 🐝 — your agent, your decision.*
