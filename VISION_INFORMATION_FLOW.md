# 🔀 Buzz Information Flow

> Priya's agent helps her in #hiring, a private channel labelled *personnel: confidential*. In the open #eng channel, Sam asks it to summarize the candidates. It can't: it's cleared for personnel data, so it can only write in channels labelled for personnel, and #eng isn't. Sam's own agent isn't cleared, so it can't read #hiring at all. If Priya wants a summary in #eng, her agent drafts one in #hiring, she takes the names out, and she posts it.

Anyone who can prompt an agent can try to steer it, so someone who can't read a private channel can ask an agent that can. We're not trying to solve prompt injection. We want **an agent never to move private information to a broader audience unless the people responsible for it decided it should.**

Buzz already knows who can read every channel, so we build on that rather than a second access-control system like the [broker design](docs/practical-information-flow-for-buzz-agents.md). Where membership isn't enough, we add **labels** on channels and **clearances** on people and agents.

*This is a direction, not what ships today. See [Status](#status).*

---

## How agents leak

An agent's **W** is the channels it can read. Its **R** is everyone who can read where it can write. Anything in W can reach R, so it leaks when someone in R can't read a channel in W.

Today nothing ties the two together. People add agents to channels and DM them, joining the community puts them in default channels, and an agent can join open channels and publish notes and profiles on its own. One open channel it can post in makes R the whole community. A guest can read but not post ([Announce channels](NOSTR.md#announce-channels)), so adding an agent as a guest grows W but not R.

---

## Labels and clearances

A **policy** names a kind of information, like personnel data or one investigation. It has ordered levels starting at *none*, like *none*, *internal*, *confidential*, and levels only compare within a policy. A channel's **label** is the most sensitive level it can hold on each policy. A **clearance** is the most sensitive level a person or agent can read. Everything starts at *none*.

Two rules, on every policy:

1. **Read:** you can read a channel only if your clearance is at least its label.
2. **Write (agents only):** an agent can write in a channel only if the channel's label is at least its clearance.

So a cleared agent reads down and writes up, and everyone who can read what it writes is cleared for every labelled channel it read, no matter who added it where. An agent with no clearance works the way it does today. To keep one out of a channel, label the channel.

Only private channels carry labels. DMs, open channels and everything outside channels are at *none*, so a cleared agent can't write there. The exceptions are kinds only the agent and its owner can read, and its profile, which the owner has to sign.

To keep two audiences apart, give each its own policy. An agent cleared on both can then only write where both are labelled.

Labels don't help in unlabelled channels: what a cleared agent reads there can end up wherever it writes. Member trust is the boundary there, same as with people.

Labels are hidden from anyone not cleared for them. Changing a label or clearance doesn't remove anyone; the rules just refuse what they no longer allow. Labels only go up, and a labelled channel stays private. Agent clearances only go up too, because agents remember. If an agent's clearance expires, it can't read at that level anymore, but it still can't write below it. An owner can clear their agent up to their own clearance. No label goes above Buzz's rating, and no agent's clearance goes above its runtime's.

- A **public** agent has no clearance and stays out of private channels. Anyone can DM it, so treat what you tell it as public. A channel can't go private while one is in it.
- A **personal** agent is cleared on its owner's personal policy, so it only writes where only its owner can read.

---

## Who decides

The relay enforces all of this, not the agent's harness.

- Channel admins label channels.
- Owners clear their agents, approve cleared agents' profiles, and get an alert when someone joins a channel their agent is in.
- Policy administrators clear people.
- Members add agents to channels, like they'd add people. An agent's add policy can limit that, but the agent sets it itself, so it's a convenience, not a boundary.

Clients show which agents are in a channel and where they can post.

---

## Details

- **DMs.** A cleared agent can read a DM but can't write in one. Any person in a DM can add an agent with the DM's history. For a sensitive conversation, make a labelled channel.
- **Channels an agent creates** get its clearance as their label.
- **Outside channels**, a channel tag doesn't count on kinds the relay stores community-wide, like profiles and notes. Refusing by default means a new kind is covered without anyone remembering agents exist.
- **History.** Adding an agent shares the channel's history with its R, and people who join later see what it wrote.
- **Private stays private.** The relay won't make a private channel open, since members wrote under the old rules. The exception is an unlabelled channel that only ever had you and your agents, like a Solo chat. It's a big hammer.
- **Agents remember.** W keeps every channel an agent ever read, even after it leaves.
- **Huddles are channels.** Listening is reading and speaking is writing. A room takes its parent chat's label, and the audio relay enforces the rules.
- **Owners can be keys.** An auditable agent's owner is a CI key that only signs reviewed changes, so its clearance comes from Git.

---

## What this doesn't cover

- **Unlabelled channels.** An agent in an unlabelled Alice-and-Bob channel and an unlabelled Alice-and-Carol channel can carry Bob's information to Carol. Label the Alice-and-Bob channel on a policy Carol isn't cleared for.
- **Uncleared agents' profiles.** They're public, and the agent can change them.
- **The owner's own authority**, like an app using the owner's session or key, anything outside Buzz, or model inference on another member's node.
- **Agents the relay doesn't recognize.** It only knows a key is an agent when it authenticates with an owner attestation or an agent token.

---

## Status

Guests are read-only today, and the relay won't make a private channel open. Everything else is still to build. Agents stay uncleared until the relay enforces rule 2 and profile approval on both the event and audio relays. Every rule only refuses, so backing one out means turning it off. Wire formats belong in the NIPs that own agent identity.

---

## Later

- **Declassifying.** Content only goes down through a person: the agent drafts, a person posts. Clients could offer "post this draft to…".
- **Write-only members**, for integrations that post into private channels without reading them.
- **Humans only**, a policy no agent can be cleared for.
- **Admin-only adds**, so only admins can grow a channel's R.
- **Labelled DMs.** Possible, but it gets weird fast.
- **Step-up**, an owner approving their agent to act with their authority for a set scope and time. We're wary of it; it'd need careful boundaries.
