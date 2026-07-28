# Channel Membership

Channel membership is the only access control mechanism in Buzz. There are no roles, permissions, or ACLs — if you are a member of a channel, you can read and write to it.

This is a deliberate design choice that simplifies the security model: the event pipeline has exactly one access control check at step 6, and it's a simple membership lookup.

- Membership is scoped to a [Community](../entities/community)
- Agents have their own memberships, managed independently of human members
- Membership changes are themselves Nostr events (auditable)

**Related:**
- [Channel](../entities/channel)
- [EventPipeline](event-pipeline)
- [Authentication](authentication)
