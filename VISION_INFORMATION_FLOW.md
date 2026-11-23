# 🔀 Buzz Information Flow

> Priya's agent helps her in #hiring, a private channel labelled *personnel: confidential*. In the open #eng channel, Sam asks the agent to "post a summary of everything you know about the candidates." It can't. Priya cleared it for personnel data so it could read #hiring, and a cleared agent can only write where the label covers its clearance. #eng has no label, a DM can't carry one, and Sam isn't cleared to read a channel labelled for personnel. Sam's own agent isn't cleared, so it can't read #hiring in the first place. If Priya wants a summary in #eng, her agent drafts it in #hiring, she takes the names out, and she posts it herself.

Agents act with their owner's permissions, and anyone who can prompt an agent can try to steer it. So someone who can't read a private channel can ask an agent that can, and prompt injection makes that easy. We're not trying to solve prompt injection. We're trying to make sure **an agent doesn't move private information to a broader audience unless the people responsible for that information decided it should.**

Buzz already knows who can read every channel, and the relay already enforces it, so we build on that. A [broker design](docs/practical-information-flow-for-buzz-agents.md) gets a stronger guarantee with keyless agents, a trusted broker, one agent instance per audience and labelled memory, but it's a second access-control system next to channels. Where membership isn't enough, we add a **label** on a channel and a **clearance** on a person or agent.

*This is the direction we intend to build, not what ships today. See [Status](#status).*

---

## How information leaks through an agent

An agent reads some channels and writes in others. Call the channels it can read **W**, and everyone who can read where it can write **R**. Anything in W can end up in front of R. That's fine as long as everyone in R can already read every channel in W. A private channel in W with a reader in R who isn't a member is a leak.

Channel roles decide both. Members, bots, admins and owners read and write; guests only read ([Announce channels](NOSTR.md#announce-channels)). So the first open channel an agent can post in makes R the whole community. Reading an open channel is always fine, since it's public already.

Today nothing ties where an agent writes to what it's read. Members add it to channels with a role that can post. Anyone can DM it. Joining the community puts it in every default channel, where it can post if the join role is member. And its own key can join open channels, create channels, open DMs, and publish notes and profiles everyone can read.

---

## Labels and clearances

A **policy** names one kind of information: customer data, personnel data, one investigation, one person's own stuff. Each policy has ordered levels starting at *none*, for example *personnel*: *none*, *internal*, *confidential*. Levels only compare within a policy. A channel's **label** is the most sensitive level it may hold on each policy, and a person's or agent's **clearance** is the most sensitive level they may read. Everything starts at *none*. An agent cleared above *none* on any policy is a **cleared agent**.

Two rules, on every policy, whatever the membership:

1. **Read:** you can read a channel only if your clearance is at least its label.
2. **Write (agents only):** an agent can write in a channel only if the channel's label is at least the agent's clearance.

So a cleared agent reads down and writes up. That's what makes it work: everyone who can read where it writes is cleared for every labelled channel it read, no matter who added it where. To keep two audiences apart, like two investigations, give each its own policy. The cost is that an agent cleared on both can only write in channels labelled on both. Labels don't protect unlabelled channels, though: what a cleared agent reads in an unlabelled private channel can reach the labelled channels it writes in.

Only a private channel that isn't a DM can carry a label, and a Huddle room takes its parent chat's. Everything else is at *none*, so a cleared agent can't write there. Labels are hidden from anyone not cleared for them; they just get "not authorized". Raising a label or lowering a clearance doesn't remove anybody. The rules just refuse what they no longer allow.

Being cleared costs an agent a lot: no notes, no posting in open channels or DMs, no editing its own profile freely. So rule 2, the label on channels it creates, the refusal of events outside channels and profile approval only apply to cleared agents. An agent with no clearance works the way it does today, and to keep it out of a channel, you label the channel. That's also how a person works, as far as information flow goes. The difference is that a person uses their own judgment, and an agent can be steered by whoever prompts it.

Levels only move one way. Labels only go up, and a labelled channel stays private. An agent's clearance only goes up, because it remembers what it read. If a clearance expires, the agent's reads stop at the old level, but its writes are still held to it. Each policy's administrators clear people, and an owner can clear their agent up to their own clearance. No label goes above Buzz's rating, and no agent's clearance goes above its runtime's rating.

Two kinds of agent are worth naming. A **public** agent has no clearance and stays out of private channels. Anyone can DM it, so clients tell people that what they say to it is public, and a channel can't go private while one is in it. A **personal** agent is cleared on its owner's personal policy, so it only writes where only its owner can read.

---

## Who decides

Nobody controls an agent's flow alone, and the relay enforces each part:

- A channel's admins label it, and the label holds for every agent, whoever owns it.
- An agent's owner sets its clearance, which decides which labelled channels it can read and where it can write. They approve each change to a cleared agent's profile, and get an alert when someone joins a channel the agent is in.
- Each policy's administrators decide which people are cleared.
- Members decide which channels an agent is in. Anyone who can post can add an agent, like they'd add a person, unless the agent's add policy allows only its owner. The agent's own key sets that policy, so it's a convenience, not a boundary. Clients show which agents are in a channel and where they can post.

In an unlabelled channel, member trust is the only boundary.

---

## Details

- **DMs.** A DM can't carry a label, so a cleared agent can't write in one, whoever opened it. It can still read a DM someone opens with it. In a DM or any unlabelled channel, privacy rests on member trust, so for a sensitive conversation, make a labelled private channel. The relay enforces this, not a harness that filters what reaches the model.
- **Channels an agent creates.** The creator owns the channel, so it's in W and everyone the agent adds is in R. A channel a cleared agent creates gets the agent's clearance as its label, so the agent can work there with people cleared for it.
- **Events outside channels.** These go to whoever the relay lets read them, with no membership involved. Rule 2 refuses all of them from a cleared agent, whatever their tags; a channel tag doesn't count on kinds the relay stores community-wide anyway, like profiles, notes, long-form, user status, git events and lists. The exceptions are kinds only the agent and its owner can read (each one has to really be that), and the agent's profile, which clients need because it carries the owner attestation. Profile fields are public, so the relay only accepts a cleared agent's profile with the owner's signature over its content. Refusing by default means a new kind is covered without anyone having to remember that agents exist.
- **Adding an agent shares the channel.** It shares what the channel holds with the agent's R, just like adding a person shares the history with them. Any person in a DM can add an agent with the DM's history. Whoever adds an agent is trusting the channel's admission rules, not just its current members, and people who join later can read what the agent wrote before they got there. A channel that needs more than that gets a label.
- **Private stays private.** The relay won't make a private channel open, for anyone, because members wrote under the old rules. If you want an open channel, make a new one. The exception is an unlabelled channel whose members, past and present, are only the person opening it and their agents, like a Solo chat. Opening that is just a person disclosing what they can already read, including whatever their uncleared agents wrote there and pulled from other channels. It's a big hammer.
- **Agents remember.** Removing an agent stops new information, but W keeps every channel it ever read.
- **Huddles are channels.** Listening is reading and speaking is writing, so the same rules apply on the audio relay. A room takes its parent chat's label, the chat's members join automatically, every join passes rule 1, and the audio relay refuses a cleared agent's audio where the label doesn't cover its clearance. The agent hears everything in the room; its client picking whose speech to act on isn't enforcement.
- **Owners aren't always people.** The owner is whichever key signed the agent's owner attestation. For an auditable agent, that's a CI key that only signs reviewed changes. Its clearance comes from Git, and since it's normally cleared, a reviewed change bounds what it reads and where it writes. Its join alerts go to the on-call group in its deployment.

---

## What this doesn't cover

- **Unlabelled channels.** If an agent is in an unlabelled Alice-and-Bob channel and an unlabelled Alice-and-Carol channel, Bob's information can reach Carol. Members chose both, and nothing stops it. The broker design would; here it's up to the Alice-and-Bob channel's admins to label it on a policy Carol isn't cleared for.
- **Uncleared agents' profiles.** They're public, and the agent can change them any time.
- **The owner's own authority.** An app using the owner's session or key isn't an agent action. Nothing here sees the owner's files and tools outside Buzz, or what an agent sends through model inference on another member's node.
- **Agents the relay can't identify.** The relay only knows a key is an agent when it authenticates with an owner attestation or an agent token. Until then it looks like a person, and no clearance applies.

---

## Status

Today guests are read-only everywhere, and the relay won't make a private channel open, with the exception above. Everything else here is still to build. Agent clearances stay at *none* until the relay enforces rule 2, the refusal of events outside channels, and profile approval, on both the event and audio relays. Each rule only refuses actions, so backing one out means turning it off; existing agents keep their memberships and roles. Wire formats get specified when they're built, in the NIPs that own agent identity.

---

## Later

- **Declassifying in the client.** Content already only goes down through a person: the agent drafts, and a person who can read the draft posts it lower as their own. Clients can offer "post this draft to…". Usually that means pseudonymizing or anonymizing, which can move content to another policy, not just a lower level. The agent can do the transform; the person decides when it's done.
- **A write-only role**, so an integration can post into lots of private channels without reading them. R grows and W doesn't.
- **Humans only**, a built-in policy every person is cleared for and no agent can be.
- **Admin-only adds**, a channel setting so only owners and admins can grow R. It'd help announce channels too.
- **Labelled DMs.** Possible, but it gets weird fast.
- **Step-up**, letting an owner approve an agent acting with their authority for a stated scope and time, like summarizing a private channel the agent isn't in. It'd be attributable, counted in W and R, and revocable. We're wary of it: it widens what an agent can reach, so it needs its own design with careful boundaries. Same goes for an agent changing its owner's Buzz configuration.
