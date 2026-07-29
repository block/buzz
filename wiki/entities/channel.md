# Channel

Channels are communication spaces within a [Community](community). Buzz supports three channel types:

## Stream
Slack-like real-time chat. Mandatory topics on creation. Supports sub-replies (thread-like) and has zero-notification default to reduce noise.

## Forum
Discourse-like async long-form posts. Flat replies. Designed for asynchronous discussion rather than real-time chat.

## Direct Messages
1:1 and group DMs (up to 9 participants). Private communication outside of stream/forum channels.

**Channel membership is the only access control gate** in Buzz. If you're a member of a channel, you can read and write to it. There are no roles, permissions, or ACLs beyond membership.

**Related:**
- [ChannelMembership](../concepts/channel-membership) — access control model
- [Community](community) — parent tenant
- [NostrEvent](nostr-event) — every channel action is an event
