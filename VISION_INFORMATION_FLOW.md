# 🔀 Buzz Information Flow

> Priya's agent helps her in #hiring, a private channel classified *personnel: confidential*. In the open #eng channel, Sam asks it to summarize the candidates. It can't: it's cleared for personnel data, so it can only write in channels classified for personnel, and #eng isn't. Sam's own agent isn't cleared, so it can't read #hiring at all. If Priya wants a summary in #eng, her agent drafts one in #hiring, she takes the names out, and she posts it.

Anyone who can prompt an agent can try to steer it, so someone who can't read a private channel can ask an agent that can. We're not trying to solve prompt injection. We want to control agents' ability to expose private information to an audience that isn't intended to have it.

Buzz already knows who can read every channel, so we build on that rather than a second access-control system like the [broker design](docs/practical-information-flow-for-buzz-agents.md). Where membership isn't enough, we add **classifications** on channels and **clearances** on people and agents.

*This is directional and not yet implemented. See [Status](#status).*

---

## How agents leak

An agent's **W** is every channel it can read or has read. Its **R** is everyone who can read where it can write. Anything in W can reach R, so it leaks when someone in R can't read a channel in W.

Today there is no real support for managing W and R for an agent, which is to say that all agents are essentially public conduits. People add agents to channels and DM them, and an agent can join open channels and publish notes and profiles on its own. One open channel it can post in makes R the whole community.

---

## Classifications and clearances

A **policy** names a kind of information, like personnel data or one investigation. It has ordered levels starting at *none*, like *none*, *internal*, *confidential*, and levels only compare within a policy. A channel's **classification** is the most sensitive level it can hold on each policy. A **clearance** is the most sensitive level a person or agent can read. Everything starts at *none*.

Two rules, on every policy:

1. **Read:** you can read a channel only if your clearance is at least its classification.
2. **Write (agents only):** an agent can write in a channel only if the channel's classification is at least its clearance.

So a cleared agent reads down and writes up, and everyone who can read what it writes is cleared for every channel it read while that channel was classified, no matter who added it where. An agent with no clearance works the way it does today. To keep one out of a channel, classify the channel.

Only private channels carry classifications. DMs and open channels are always at *none* – this means they're all readable to any agent that's given access, but an agent with any clearance at all won't be able to write into them. Similarly, the global event space – i.e., all events posted without a channel – is also effectively treated as *none* and unwritable by agents with a clearance. (The exceptions to this rule are a DM whose only members are the agent and its owner, kinds only the agent and its owner can read, and the agent's profile, which the owner has to sign.)

To keep two audiences apart, give each a set of channels with its own policy. An agent cleared on both can then only write where both are classified.

In unclassified private channels anything an agent reads can still end up wherever it writes. Member trust is the boundary there, same as with people.

Classifications are hidden from anyone not cleared for them. A classification protects from when it's set: classifying a channel or raising its level removes the agents not cleared for it, but what they already read isn't covered. Otherwise, changing a classification or clearance doesn't remove anyone; the rules just refuse what they no longer allow. Once set, a classification can only go up in level, except that a policy administrator can correct a mistake. Channels that carry classifications must always remain private, even if we decide to relax the rule against making private channels open generally. Agent clearances only go up too, because agents remember. If an agent's clearance expires, it can't read at that level anymore, but it also still can't write below it. An owner can clear their agent up to their own clearance. No classification goes above Buzz's rating, and no agent's clearance goes above its runtime's.

---

## Relay enforcement

The relay enforces all of this, not the agent's harness or any other client. The one exception is runtime ratings (see [What this doesn't cover](#what-this-doesnt-cover)).

- Channel admins classify channels, up to their own clearance.
- Owners clear their agents, approve cleared agents' profiles, and get an alert when someone joins a private channel their agent is in.
- Policy administrators clear people and correct mistaken classifications.
- A workflow's actions count as writes by whoever saved it, so a person's workflow is that person's call, and webhooks and other off-relay targets are at *none*.
- Members add agents to channels, like they'd add people. An agent's add policy can limit that, but the agent sets it itself, so it's a convenience, not a boundary.

Clients show which agents are in a channel and where they can post, and may have UI to warn users of the presence of agents in situations they might otherwise presume are private (like DMs).

---

## Further notes

- **Channels an agent creates** get its clearance as their classification.
- **Outside channels**, a channel tag doesn't count on kinds the relay stores community-wide, like profiles and notes. Refusing by default means a new kind is covered without anyone remembering agents exist.
- **History.** Adding an agent shares the channel's history with its R, and people who join later see what it wrote.
- **Huddles are channels.** Listening is reading and speaking is writing. A room takes its parent chat's classification, and the audio relay enforces the rules.

---

## What this doesn't cover

- **Agents the relay doesn't recognize.** It only knows a key is an agent when it authenticates with an owner attestation or an agent token. Anything running under a person's key counts as that person.
- **Anything outside the relay.** An agent can send what it read anywhere its runtime reaches: other relays, the web, its model provider. The runtime's rating is the only control there, and the relay can't check it.

---

## Status

Guests are read-only today, and the relay won't make a private channel open unless only the sender and their own agents were ever in it. Everything else is still to build. Channels stay unclassified until rule 1 covers every read path: events, audio, media, search and Git. Agents stay uncleared until rule 2 and profile approval cover every write path: events, audio, Git and the actions workflows take. Every rule only refuses, and if enforcement is turned off, classified channels fail closed: nobody reads them, and cleared agents can't write. Wire formats belong in the NIPs that own agent identity.

---

## Possible later extensions

- **Declassifying.** Content only goes down through a person: the agent drafts, a person posts. Clients could offer "post this draft to…".
- **Write-only members**, for integrations that post into private channels without reading them.
- **Humans only**, a policy no agent can be cleared for.
- **Admin-only adds**, so only admins can grow a channel's R.
- **Step-up**, an owner approving their agent to act with their authority for a set scope and time. We're wary of it; it'd need careful boundaries.
