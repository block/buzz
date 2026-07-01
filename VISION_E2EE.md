# 🔐 Buzz E2EE — Private means private

> A public relay helps a community form around an open-source project. Maintainers, contributors, guests, and agents work in the same place: channels, DMs, forums, code, canvases, workflows. Some rooms are public by design. Some are not. When someone creates a private DM or an encrypted channel, the promise is simple: the relay can deliver it, store it, back it up, and prove who sent it — but it cannot read it.

Buzz is the relay as workspace. End-to-end encryption is how the workspace stays yours even when the relay is run by someone else.

For many communities, server-visible history is the right tradeoff. Companies with retention, eDiscovery, regulated supervision, or local legal constraints may need plaintext relays. Buzz must support them. E2EE is therefore a **community/relay capability**, not a universal mandate.

But for public relays, community relays, open-source projects, friend groups, research collectives, and any group that chooses sovereignty over operator visibility, encrypted conversation should be the encouraged default. A hosted relay should not require a leap of faith. A user should not have to wonder which operator, admin, backup system, search index, workflow engine, or database reader can read their private conversation. The answer should be: only the cryptographic members of the room.

---

## The Promise

Private means private.

- The relay authenticates users, enforces membership, routes events, fans out live updates, stores ciphertext, and maintains the audit trail.
- Clients hold the keys that read the conversation.
- Adding someone is explicit. Removing someone cuts off future access.
- A newly added member does not receive old history unless existing members intentionally re-share it.
- An agent that can read an encrypted room is visibly a member of that room, with its own key material and revocation path.

The relay remains useful. It still gives Buzz the workspace shape: channels, DMs, presence, reactions, threads, media, git, workflows, agents, and audit. It just stops being the entity that can read every private word.

---

## No False Promises

E2EE is not magic erasure.

If a member already received and decrypted a message, cryptography cannot make them forget it. Removing a member protects the future: new epoch, new keys, no new plaintext. It does not revoke memory, screenshots, exported logs, or ciphertext already decrypted on a device.

Buzz should say that plainly. The promise is future secrecy and intentional membership, not retroactive mind-wipe.

---

## Optional by Law, Encouraged by Values

Some communities cannot legally use E2EE for business communication. Some operators may choose server-side retention. Some deployments are internal tools where the relay operator and the community are the same trust boundary.

Buzz should support three policy modes at the community boundary:

| Mode | Meaning |
|---|---|
| `disabled` | Encrypted conversation events are rejected; clients hide E2EE affordances. |
| `optional` | Communities/channels may choose plaintext or encrypted operation. |
| `required` | New private channels and DMs must be encrypted; plaintext private rooms are not created. |

In a single-community relay, this is a relay setting. In a multi-community deployment, it is resolved from the host-derived community. One operator may host a regulated community with E2EE disabled and a public community with E2EE required on the same shared infrastructure without weakening either boundary.

The URL is the community. The community chooses the privacy model.

---

## What Users See

A channel can be marked encrypted. The UI explains the consequence in plain language:

- message bodies are unreadable to the relay;
- server-side search and content workflows are unavailable unless an agent/member decrypts locally;
- membership changes rotate future keys;
- new members start from now unless history is shared;
- agents are listed as cryptographic members when they can read.

There is no separate encrypted app, no separate identity, no separate workspace. The same Buzz surfaces work, with honest capability changes where plaintext was previously required.

---

## What Operators See

Operators choose the community policy and publish it in relay metadata. If E2EE is disabled, the relay does not advertise encrypted conversation support and rejects encrypted conversation kinds. If E2EE is enabled, the relay still sees and enforces the metadata it needs to run the workspace:

- community and channel identity,
- authenticated publisher,
- membership and delivery scope,
- event kind,
- thread/reaction references when clients choose to make them public,
- timing and size metadata,
- media blob metadata unless attachments are separately encrypted.

Operators do not receive plaintext message bodies for encrypted rooms.

---

## Agents Are Members, Not Exceptions

Agents are powerful because they sit in the same workspace as humans. In encrypted rooms, that must stay true cryptographically.

An agent that can summarize, search, or respond inside an encrypted room is a member with keys. It should be visible in the member list. Removing it should rotate the room forward. Hosted agents are still trusted compute: the relay may not read the room, but the agent runtime can once admitted. Buzz should make that trust boundary obvious instead of pretending it disappears.

---

## The Point

Buzz gives communities a workspace they can own. E2EE lets them use hosted or public infrastructure without surrendering private conversation to that infrastructure.

No compromise means: still a relay, still searchable where plaintext is chosen, still agent-native where agents are admitted, still one workspace — but private rooms are truly private.

*Buzz 🐝 — the relay is the workspace; the keys belong to the room.*
