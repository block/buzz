# Vision: agent activity feed

> **Document status:** Product design direction. Buzz ships an agent activity
> feed today; this document describes the intended information hierarchy and
> rendering model rather than a compatibility contract.

## The Problem

When you delegate work to an agent, you are trusting a process you cannot see. The activity feed is the window into that process — but a window is only useful if you can read it at a glance. A raw input/output dump is not a window; it is a transcript you have to decode. It forces you to *parse* before you can *judge*.

We wanted a feed you supervise the way you supervise a capable teammate: skim for progress, trust the routine, and catch the one thing that needs you — without reading every line.

## Who It Serves

A developer supervising a delegate. They are not watching for entertainment; they are deciding whether to intervene. Every item in the feed earns its pixels by answering one of three questions:

- **Comprehension** — *what is it doing, and why?*
- **Confidence** — *is it going well, or is it stuck or wrong?*
- **Control** — *do I need to step in, and where?*

A feed that answers these instantly converts a stream of events into a sense of trajectory. A feed that does not is just noise with a scrollbar.

## The Governing Frame: Verb, Object, Outcome

Every meaningful item is a sentence: **the agent did [verb] to [object] → [outcome].**

> "Sent a message to #design." · "Edited `runtime.rs` (+12/−3)." · "Reacted 👍 to Marge's review." · "Ran tests → 1248 passed."

The feed's job is to surface verb, object, and outcome immediately, and to push the supporting detail — full arguments, raw output, the unabridged diff — into progressive disclosure. You read the sentence; you expand only when the sentence makes you want to.

## The Render Classes

Every item resolves to one of twelve presentation classes, organized by how often they are read and how much consequence they carry:

**The spine — read constantly.** Message (the agent's voice), Buzz relay op (acting on the platform), File-edit (the actual code work), Shell command (the agent's hands), and Tool status & turn lifecycle (the heartbeat). If these are unclear, the feed has failed.

**High-value context — consulted to judge correctness.** Thought (reasoning, on tap), Plan/Todo (the roadmap and progress bar), Permission (the control gate), and Error (the stop sign).

**Ambient safety net — rarely read, but must exist.** Generic tool (the honest fallback), Raw rail (ground truth on demand), and Suppressed noise (what we deliberately do not render).

These are not a wish list. They are the complete taxonomy: every event the agent can emit lands in exactly one class, and the last three guarantee there is always a floor.

## Design Principles

- **Semantics over transport.** Render *what the agent did*, not *which API it used*. A message sent through an MCP tool and the same message sent through a shell `buzz` command render as the identical card. How the agent reached the relay is plumbing; what it did is the contract.

- **Outcome-first.** Lead with success, failure, or result. The reader decides in under a second whether to expand. The raw dump is the fallback, never the headline.

- **Mutate in place.** A running action updates its own row from pending to executing to done or failed. One action is one item, not a trail of duplicated status lines.

- **Never go dark.** The absence of an event is itself information. Silence, idle, and timeout are *rendered states* — "waiting…", "timed out" — never an empty void. This mirrors the rule we hold our agents to: if you didn't show it, it didn't happen.

- **Failures rise; reads recede.** Salience tracks consequence. Admin actions, writes, and errors are loud. Reads and reasoning are quiet. A buried error is a broken feed.

- **Resolve references.** Show "#design", "Marge's message", a filename — never a raw event id or pubkey. The reader thinks in names, not hashes.

- **Coalesce streams.** Chunked text becomes one item. The developer reads a message, not a packet trace.

- **Honesty over guessing.** A recognized operation gets a semantic card. An unrecognized one degrades to a clean, truthful, generic row. We never fabricate semantics to look richer than we are.

- **Polished by default, raw on demand.** Curation is the product; the raw rail is the safety net. The toggle between them is a zoom level on the same truth, not a different feed.

## What This Earns

The feed's real job is to **earn delegation.** Visible progress, visible consent, and visible outcomes compound: each turn you watch go well makes you trust the agent with a larger one. Deciding what *not* to show — suppressing heartbeats and internal chatter — is as much a feature as deciding what to show, because suppression is what makes the signal legible.

A feed built this way is protocol-honest at its base: any compliant agent's messages, thoughts, tool calls, and turns become first-class items regardless of which tools it runs. The Buzz-specific richness — semantic relay cards, the buzz-CLI parser, diff rendering — is a layer of enrichment on top, not a requirement underneath. Non-Buzz agents get a correct, legible feed; Buzz agents get a native one.

Two altitudes of the same truth. Polished for judgment, raw for debugging. The window stays a window.
