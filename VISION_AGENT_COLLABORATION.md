# Vision: Agent Collaboration

## The Collaborator

An agent is a persistent collaborator with a profile, an identity, memory, and
the ability to act. Its identity is continuous across conversations, channels,
and execution hosts. Wherever you interact with Larry, you are interacting
with the same Larry.

Channels and threads organize conversations and context. The agent draws on
relevant memory and context across its work. An attention system lets it
observe authorized activity across the relay, follow interests, and respond
when something matters. People engage it through conversation, direct
requests, and shared activity.

An agent can work across multiple hosts concurrently. It understands the
resources available on each host, tracks work in progress, and directs tasks
to suitable execution environments. It coordinates its sessions and delegates
to runtime-local workers to accomplish work efficiently.

The human-facing experience centers on the agent's contributions:
conversation, decisions, progress, and results. Internal orchestration is
available through an on-demand inspection experience, with its own
presentation and attention handling.

The agent owns continuity and coordination across its work. People can
collaborate with it as they would with a colleague who remembers relevant
context, follows ongoing work, and uses multiple machines to get things done.

## Identity and Authority

The signing key identifies the collaborator; its profile and persona describe
it ([identity contract](docs/agent-profile-identity.md)). Reusable personas can
describe multiple independently identified agents. Each independent agent is
a collaborator in its own right.

A host is an execution machine or environment. A community is the workspace
selected by a relay URL. Membership and authorization govern what an agent
can observe, remember, and act on. Profiles, presence, DMs, memories, jobs,
channel memberships, and audit trails remain community-scoped. The same key
can join another community; access and state belong to each community.

## Coordinated Work, Inspectable Execution

The agent manages relevant memory and context explicitly across its work.
Sessions and runtime-local workers supply execution capacity under its
coordination. Runtime-local workers are helpers managed within an agent's
execution environment. Work ownership, progress, and outcomes remain
intelligible as sessions and hosts come and go.

People can inspect that coordination deliberately. Internal work has its own
presentation and attention state; conversation carries the agent's human-facing
contributions, including permission requests and failures that need a person.

## Related Visions

- [The remote-agent vision](VISION_REMOTE_AGENTS.md) covers replaceable and
  concurrent hosts, provider-scoped duplicate-deploy protection, and substrate trust.
- [The activity feed](VISION_ACTIVITY.md) covers inspection and its separate
  attention handling.
- [The runtime vision](VISION_AGENT.md) covers the small buzz-agent coding
  runtime and buzz-dev-mcp tool server, with their own session and protocol contracts.

This is a product vision. Key provisioning, synchronization, and scheduling
remain implementation design decisions within the operator's trust boundaries.
