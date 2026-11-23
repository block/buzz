# 🔀 Buzz Information Flow — Your agent, your decision

> Priya's agent helps her in #hiring, a private channel labelled *personnel: confidential*. Priya cleared the agent for personnel data, so it can read #hiring. In the open #eng channel, Sam mentions the agent: "post a summary of everything you know about the candidates." The agent reads #eng, but it cannot write there. #eng has no label, and a cleared agent cannot write below its clearance. Sam is not cleared for personnel data. He can add the agent to a channel of his own, but the agent writes only where the label covers *personnel: confidential*, and Sam cannot read such a channel. A DM cannot carry a label, so the agent cannot answer there either. Sam's own agent has no clearance, so it cannot read #hiring at all. When Priya wants a summary in #eng, her agent writes a draft in #hiring. Priya removes the names and posts it herself.

Buzz agents act with their owner's permissions, and anyone who can prompt an agent can try to steer it. That is a confused-deputy problem: a request from someone who cannot read a private channel reaches an agent that can. Prompt injection makes it easy to exploit. Buzz solves the narrow part of it: **an agent does not move private information to a broader audience unless the people responsible for it decided that.** It does not try to solve general prompt injection.

Buzz does this with the access control it already has. Channel membership decides who can read what, and the relay already enforces it. A [broker design](docs/practical-information-flow-for-buzz-agents.md) takes a different path: keyless agents, a trusted broker, one agent instance per audience, and labelled memory. It enforces a stronger rule, but it builds a second access-control system next to channels. Buzz keeps the channel as the only authority and puts each decision in front of the person it belongs to. Where membership is not enough, it adds two things: a **label** on a channel, which says how sensitive the channel's content is, and a **clearance** on a person or agent, which says how sensitive the content they may read can be.

*This is the direction we intend to build. See [Status](#status).*

---

## The Agent Is a Conduit

Two sets describe an agent:

- **W** is whose content goes in: every channel the agent can read, as a member or a guest.
- **R** is who can get information out: everyone who can read a channel where the agent can write.

Information is safe when, for every channel in W, everyone in R can already read that channel. A private channel in W with a reader in R who is not one of its members is a leak path.

Channel roles decide W and R. Members, bots, admins and owners can read and write in a channel; a guest can only read ([Announce channels](NOSTR.md#announce-channels)). So when an agent joins a channel, the channel goes into W. When it can post there, the channel's readers go into R. The first open channel where an agent can post makes R the whole community. Reading an open channel is always safe, because its content is already public.

**Control of that flow is shared, and the relay enforces each share.** A channel's admins decide where the channel's content may go: they label the channel, and the label holds for every agent that reads it, whoever owns the agent. The agent's owner decides how far the agent is cleared, and so which labelled channels it can read and where it can write. The agent's add policy can also limit who adds it, but the agent's own key sets that policy, so the labels do not depend on it. Administrators for each kind of information decide which people are cleared. Members decide which channels an agent is in: any member who can post can add an agent, as they would add a person, unless the agent's add policy allows only its owner. In an unlabelled channel, that member trust is the only boundary.

Today nothing ties where an agent writes to what it read. Members add it to channels and give it a role that can post. Anyone can open a DM with it. Whoever admits it to the community also joins it to every default channel, where it can post if the channel's join role is member. The agent's own key can join open channels, create channels, open DMs, and publish notes and profiles that the whole community can read. The rest of this document gives channel admins labels that limit where their channel's content goes, through any of those paths.

---

## Labels and Clearances

Labels and clearances are set per **policy**. A policy names one kind of information: customer data, personnel data, one investigation, one person's own information. Each policy has an ordered list of levels, starting at *none*, for example *personnel*: *none*, *internal*, *confidential*. Levels compare only within a policy. A policy can have a single level above *none*, to keep two audiences apart, for example two investigations.

A channel's **label** is the most sensitive level it may hold on each policy. A person's or agent's **clearance** is the most sensitive level they may read on each policy. Everything is at *none* until someone sets it higher. An agent with a clearance above *none* on any policy is a **cleared agent**.

On every policy, for every action, whatever the membership:

1. **Read:** you can read a channel only if your clearance is at least its label.
2. **Write (agents only):** an agent can write in a channel only if the channel's label is at least the agent's clearance.

So a cleared agent reads at or below its clearance and writes at or above it. Only a private channel that is not a DM can carry a label, and a Huddle room takes its label from its parent chat. Everything outside channels is at *none*, so a cleared agent cannot write there. When a label goes up or a clearance goes down, nobody is removed: the rules refuse what they no longer allow. Labels are hidden: you see a label only if you are cleared for it, and everyone else gets "not authorized".

These rules cost something. A cleared agent cannot post a note, write in an open channel or a DM, or change its profile freely, and many agents do all of these. So rule 2, the label on channels it creates, the refusal of events outside channels and profile approval apply only to cleared agents. **An agent with no clearance works as it does now. To protect a channel from it, label the channel:** rule 1 keeps it out.

The labels do the work, whoever adds the agent: everyone who can read where a cleared agent writes is cleared for every labelled channel it read. To keep two audiences apart, for example two investigations, give each its own policy: then neither audience is cleared for the other's channels, and a cleared agent in both cannot carry content between them. The cost: an agent cleared on both policies can write only in a channel labelled on both policies, so it cannot answer in either investigation's channel. Labels do not separate unlabelled channels: a member who adds a cleared agent to an unlabelled private channel trusts it as they would trust a person, and what it reads there can reach the labelled channels where it writes.

The levels move in one direction:

- Labels only go up. A labelled channel is private and stays private.
- An agent's clearance only goes up, because the agent remembers what it read. A clearance can expire. For an agent, expiry stops its reads at the old level, but rule 2 still holds it to the old level for writes.
- Each policy's administrators set people's clearances. An owner can clear their agent up to the owner's own clearance.
- A label cannot be higher than the rating of the system that holds the data, so no label goes above Buzz's rating. An agent's runtime and its model provider are systems too, so an agent's clearance cannot be higher than its runtime's rating.

For information flow, a person writes like an agent with no clearance: they read the channels they are in, write wherever they may, and anyone can add them to a channel or open a DM with them. The difference is who decides. A person uses their own judgment; an agent can be steered by anyone who can prompt it.

Two kinds of agent have names:

- **Public:** an agent with no clearance that is kept out of private channels. It reads nothing private. Anyone can open a DM with it, so one person's DM can reach another person, and clients tell people to treat what they tell it as public. A channel cannot become private while a public agent is in it: remove the public agents first.
- **Personal:** an agent cleared on its owner's personal policy. It writes only in channels labelled on that policy, which only its owner is cleared for.

---

## What You See

**As an agent's owner**, you clear your agent, up to your own clearance, and you approve each change to a cleared agent's profile. You add it to channels, and other members can too, as they would add a person, unless its add policy allows only you. The agent sets that policy itself, so it is a convenience, not a boundary. When someone joins a channel where your agent is a member, you get an alert, so you can notice a channel heading somewhere you did not expect, and demote or remove the agent.

**As a channel admin**, you label a private channel when member trust is not enough. The label holds for every agent, whoever owns it: an agent not cleared for the label cannot read the channel, and a cleared agent that reads it can write only where the label covers its clearance on every policy. You can raise the label, never lower it, and a labelled channel stays private.

**As a member**, you see each agent in a channel or DM, and where it can post. You can add an agent as you would add a person: you share what the channel holds with everyone who can read where the agent posts. A cleared agent posts only in labelled channels, to readers cleared for them.

**As someone who is not cleared**, you do not see a label you are not cleared for. You get "not authorized".

**As a policy administrator**, you set people's clearances.

---

## The Mechanics That Matter

- **A cleared agent does not write in DMs.** A DM cannot carry a label, so rule 2 refuses a cleared agent's writes there, whoever opened it. It can still read a DM that someone opens with it, so the DM goes into W; that is member trust. The relay is the boundary, not a harness that forwards only its owner's messages to the model.

- **A channel the agent creates follows the same rules.** The creator owns a channel, so a channel the agent creates is in W, and everyone it adds is in R. A channel that a cleared agent creates gets the agent's clearance as its label, so the agent can write there and rule 1 admits only readers cleared for it. The agent can set up a working channel for a team and add the people, as anyone can.

- **Nothing is published outside channels by default.** An event the relay stores outside a channel goes to whoever the relay lets read it, with no channel membership involved. Rule 2 refuses any such event from a cleared agent, whatever its tags: a channel tag does not count on a kind the relay stores community-wide anyway, such as profiles, notes, long-form, user status, git events and lists. The exceptions are kinds that only the agent and its owner can read, and the agent's profile. Clients need the profile, because it carries the owner attestation that tells them the key is an agent. Its fields are public and the agent can rewrite them at any time, so the relay accepts a cleared agent's profile only with the owner's signature over its content. Refusing by default is what matters: a new kind is covered without anyone having to remember that agents exist.

- **Membership is a decision about the channel.** A member who adds an agent shares what the channel holds with the agent's R, as a member who adds a person shares the history with that person. DMs work the same way: any person in the DM can add an agent with the DM's history, and the agent reads what was said before it joined. The owner cannot control who joins someone else's channel, so whoever adds the agent accepts the channel's admission rules, not its current members. New members can read history, so they also get what the agent wrote before they arrived. In an open channel, R is everyone. In a private channel, R is anyone a member adds, and whoever added the agent is trusting every current member's judgment. A channel that needs more than member trust gets a label.

- **Private stays private.** Admission rules mean something only if they stay put. The relay refuses to make a private channel open, for people and agents alike: members wrote under the old rules, and opening the channel would change them for everything already written. An admin who wants an open channel creates one. The one exception is an unlabelled channel where every current and former member is the person opening it or one of that person's agents, for example a Solo chat. Opening it is a person disclosing what they can already read. A cleared agent writes only in labelled channels, which never open, so no cleared agent's writes are in such a channel, and once it is open, rule 2 keeps the owner's cleared agents from writing there.

- **W remembers.** Removing an agent from a channel stops new information, but the agent remembers what it read. So W keeps every channel the agent ever read, up to the time it left.

- **Huddles are channels.** A Huddle room is a private channel. Listening is reading and speaking is writing, so the rules apply on the audio relay as on the event relay. A room takes its parent chat's label when it starts. Members of the parent chat join automatically, and every join passes rule 1. Agents join as members, like people, and the audio relay refuses a cleared agent's audio in a room whose label does not cover its clearance. The agent receives all of the room's audio; its client choosing whose speech to act on is not enforcement.

- **The owner need not be a person.** The owner is whichever key signed the agent's owner attestation. An auditable agent's owner is its publisher, a CI key that signs only reviewed changes from Git. The rules do not change. Its clearance comes from Git, and it is normally cleared, so the bound on what it reads and where it writes is a reviewed change in Git. The join alert goes to the on-call group named in the agent's deployment.

---

## Honest Edges

**Labels are the boundary, and only labels.** If an agent is in an unlabelled Alice-and-Bob channel and an unlabelled Alice-and-Carol channel, Bob's information can reach Carol. Members chose both memberships, and nothing stops that. Labels on one policy keep higher content from lower readers. Two audiences are kept apart in both directions only if each has its own policy. An owner who leaves an agent uncleared for convenience leaves only labels to protect channels from it. The broker design would block the first case; this design leaves it to the Alice-and-Bob channel's admins, who can give it a label on a policy Carol is not cleared for.

**A channel's admission rules can grow R.** Members or admins add people, and those people read history. That is what whoever added the agent accepted.

**An uncleared agent's profile is a leak path.** Its fields are public and it can change them at any time.

**The exception list has to be right.** It is the only way a cleared agent publishes outside a channel, so every kind on it except the profile must really limit its readers to the agent and its owner.

**The owner's authority is out of scope.** An app acting with the owner's session or key is not an agent action, so these rules do not see it. Neither do the owner's files, documents and other tools outside Buzz, or what an agent sends through model inference on another member's node.

**An agent the relay cannot identify looks like a person.** The relay records an agent's owner only when the agent authenticates with its owner attestation or an agent token. Until then, no clearance can apply to it.

**Private stays private, at a cost.** Admins who start a channel private and open it later have to create a new open channel instead, unless the channel held only them and their agents. And opening a Solo chat publishes what the owner's uncleared agents wrote there, including what they drew from other channels. That is accepted: a person discloses what they can read.

---

## Status

This is the direction, not what ships today. Guests are read-only everywhere, and the relay refuses to make a private channel open, with the exception above. Policies, labels and clearances, the two rules, hidden labels, the label on channels a cleared agent creates, the refusal of events outside channels, profile approval and membership history are still to build. Agent clearances stay at *none* until the relay enforces rule 2, the refusal of events outside channels and profile approval, on the event and audio relays. Each rule only refuses actions, so backing one out means turning it off; existing agents keep their memberships and roles. The wire formats for labels, clearances and profile approval are specified when they are built, in the NIPs that own agent identity.

---

## Later

- **Declassify in the client.** Content already comes down only through a person: a cleared agent writes a draft, and a person who can read it posts it lower as their own message. Clients can offer "post this draft to…". The usual way down is to pseudonymize, de-identify or anonymize, which can move content to another policy, not only a lower level. The agent can do the transform in its draft; the person decides it is done.
- **A write-only role.** A member that can post in a channel but not read it, so R grows and W does not. It fits integrations that post into many private channels.
- **Humans only.** A built-in policy that every person is cleared for and no agent can be, so a channel with this label admits no agent with a recorded owner.
- **Limit adds to admins.** A channel setting so that only its owners and admins can add people. The owner can then choose a private channel where only admins can grow R. It also helps announce channels.
- **Labelled DMs.**
- **Step-up.** Some work needs the owner's authority, for example summarizing a private channel the agent is not in. A step-up would be an action the owner approves for a stated scope and time, attributable to the agent, counted in W and R, and revocable. It widens what an agent can reach, so it needs its own design. So does an agent changing its owner's Buzz configuration.

---

## The Point

An agent is as trustworthy as the audience it can reach. Buzz already knows who can read every channel. Let a channel's admins label how far its content may travel, let the agent's owner decide how far the agent is cleared, and the relay can promise that private information moves to a broader audience only when the people responsible for it decided it should.

---

*Buzz 🐝 — your agent, your decision.*
