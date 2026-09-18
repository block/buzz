# Buzz Mobile

Buzz Mobile is a complete, offline-first Buzz client for iOS and Android. It can be used in concert with desktop Buzz, or as one's first and only Buzz client.

This document describes its intended behavior and architecture. For that purpose, this document takes priority over any other document or code. Not all of the documented intents are realized in the implementation yet.

When writing a spec or implementing significant changes or tests, check them for consistency with this doc. If not yet consistent, it may be this doc that needs to change, but that change should be reviewed by this doc's owner.

## Experience principles
- **Offline first.** Show cached content immediately; support reading, composing, and queuing actions regardless of connectivity. Sync in the background without disrupting use. Make unavailable content and prolonged sync problems clear.
- **Responsive UI.** The user interface does not hang or stutter, regardless of the scale of a community or number of communities joined. Expensive or slow operations happen in parallel with UI feedback and animation. Actions are reflected in UI instantly and optimistically.
- **Don't lose user-generated data.** Drafts and pending actions survive restarts, app updates, and cache rebuilds. Important pending actions (e.g. undelivered sent messages) are clearly identified visually and offer affordances for retrying or recovering the uncommitted data.
- **Don't trust relays.** Their behavior is outside our and users' control. Critical user intents (leaving a community, stopping notifications from a relay, etc.) should never be blocked on a response from a relay. The behavior or performance of one relay that the client is connected to should not degrade the experience of using other relays or non-relay-dependent functionality in the app, including by adding latency.
- **Accessibility.** The app supports platform accessibility features, including screen readers, text scaling, and reduced motion.

## Relationship to other clients

Mobile and desktop follow shared rules for behavior such as channel ordering, unread state, and autocomplete. Presentation and interaction should suit each platform.

The desktop implementation is a reference we can use to inform defining those rules, but it contains bugs and unintended behavior and is not itself the specification. Written intent takes priority over implementation precedent. A clearer vision or spec developed for mobile can establish the intended shared behavior for desktop too.

## Architecture

The following are preferred architectural approaches in service of the experience principles. They can evolve when a better approach is established.

- Verify events on receipt and fold them into the persistent store. Avoid re-verifying events already verified in the current persistent store. UI subscribes to the persistent store, never to events.
- Avoid migrations of the persistent store. It should be fully derived from received events and should be discarded as needed, such as when an app update changes the schema nontrivially. Uncommitted user-generated data that cannot be lost, including drafts and pending actions, should be kept in a separate, minimally scoped persistent store (or multiple).
- Avoid blocking the UI isolate. E.g. with expensive work such as signature verification, JSON or Markdown parsing, writing to the persistent store, or even unbounded text layout.
- Keep background work, storage, and network use bounded, with attention to battery and data consumption.

## Verification

- Tests should be fast and deterministic (not flaky), including end-to-end tests. Dependencies should be replaced with fast, deterministic mocks in tests where possible.
- After a code change, neither the developer nor CI should rerun tests that cannot have been affected by the change. E.g. a pure mobile client change shouldn't rerun relay or desktop client tests, and vice versa.
- Tests and implementations should reflect the intended behavior described here and in applicable specs.

## Release cycle

- We want to minimize the average time elapsed between (a) merging a change and (b) the change appearing in Comp Portal and the app stores.
- We should always have a release `Waiting for Review` / `In Review` in the iOS App Store. Immediately a release is approved, we submit a new one.
- We should build and upload release candidates frequently. Every RC should go to Comp Portal and TestFlight. Building and distributing an RC from a commit on main should require only a single click/command. When the time arrives to submit the next release to public app stores, there should already be a suitable RC uploaded and dogfooded.

## Critical functionality

### Channels and conversations

- Once fetched, the channel list remains available during refreshes and connectivity loss.
- Channel ordering and unread highlights and badges follow shared cross-client rules.
- Channel and thread conversations support reading and composing with immediate local feedback.
- Sending distinguishes pending, sent, and failed states. Sent means relay acceptance is confirmed, not that another person or device has received or read the message. Uncertain acceptance remains pending; failures that need intervention are visible.
- Retrying a pending send does not create a duplicate message.

### Unread state

- Unread highlights and numerical badges follow shared rules, including how thread activity contributes to a channel's unread state.
- Reading updates local unread state immediately, and read state synchronizes across devices.

### Autocomplete and search

- `@` autocomplete should show results instantaneously. Results and ordering must follow shared cross-client rules, including humans and agents, both remote and local, both in the channel and not.
- Search queries the relay rather than being limited to locally cached content.

### Notifications

- Message notifications show the Buzz icon, sender avatar, channel name, and message snippet, subject to privacy choices.
- Notifications respect shared preferences and mute state and avoid alerts for activity already known to be read.
- Opening a notification reaches the relevant community and conversation, including from a cold start. Inaccessible content has a clear explanation.
