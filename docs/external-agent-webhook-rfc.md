# RFC: Community-managed application agents

**Decision for Buzz maintainers.** Should a community be able to register an
externally hosted application agent once and let designated members use it
alongside their personal agents? If so, which parts of access to the agent
should the community control? [Issue #9220](https://github.com/block/buzz/issues/9220)
tracks the use case and alternatives.

## Why this fits the discussion about Buzz

The [Buzz vision](../VISION.md) puts conversations and their access rules in
the community. The [remote-agent vision](../VISION_REMOTE_AGENTS.md) ties an
agent's identity to its key, so the same agent can run on a different host.
This proposal asks whether a community should also be able to administer who
can use that identity when an external service supplies its tools and reasoning.

For example, a purchasing department could register one agent for supplier
queries. From desktop, web or mobile, an eligible buyer could find it in
Agents or New message, keep two supplier discussions in separate private
sessions, and mention the same agent in a purchasing channel it has joined.
The service would remain available when every buyer's desktop is closed.
Invoking it would grant only the business operations the service exposes.

## Proposed access model

| Permission | What it would allow |
| --- | --- |
| Discover | See the agent in the community directory and conversation pickers. |
| Use | Start or resume a private session, or invoke the agent in a conversation where both caller and agent are admitted. Adding the agent to a channel would still require permission to invite it. |
| Manage | Configure the service behind the agent and hand responsibility to another authorized operator while preserving its identity. |
| Grant or revoke access | Choose which community members or groups have each permission, including a community-wide grant where appropriate. |

Buzz already distinguishes an agent's [owner attestation](nips/NIP-OA.md)
from [relay admission](nips/NIP-AA.md). A buyer using the purchasing agent
would need neither its signing key nor its service credentials. The relay
would enforce access when a member discovers or invokes the agent and check
that a delayed reply is still authorized before publishing it. Each private
session would retain its own context and audience. A channel invocation
could use only information authorized for that channel's audience.

## Requested maintainer decision

Would maintainers support this as a community feature? If so, can existing
membership and roles express these permissions, including revocation and
administrative handover, or would Buzz need an agent-specific grant?

When a member loses permission to use an agent, should Buzz close their existing
private sessions, keep them available as read-only history, or allow a handover?
If handover is supported, who may authorize access to the private history?

A human Buzz maintainer should own that decision. If the direction is
accepted, a follow-up can define the smallest open contract an external
runtime would need to implement.
