# RFC: community-managed application agents with an open runtime contract

Status: proposal awaiting maintainer direction; not an implemented feature or an
established new standard. Tracking discussion: [#9220](https://github.com/block/buzz/issues/9220).

## User outcome and product direction

An operator registers an application agent once for a Buzz community. Eligible
members find the same deployed identity in Agents and New message, open private
conversations, and invite or mention it in permitted channels from desktop, web
and mobile. Each user can start and resume multiple independent private sessions
with that same agent. Its runtime remains available with every desktop closed.

Reasoning, tools and private runtime memory stay in an external service such as
n8n. This is a shared service for a company or department, with explicitly scoped
business capabilities. Ordinary users need no local runtime, coding harness,
client plugin or backend credential.

**The first decision is product fit:** does this model align with Buzz's current
direction, alongside personal agents? A request for discussion is not a delivery
commitment. No transport, provider or new event kind is selected in this draft.
The earlier ACP-to-HTTP design is retained below as one candidate.

This proposal follows [VISION.md](../VISION.md)'s separation between collaboration
and agent intelligence. [Remote-agent vision](../VISION_REMOTE_AGENTS.md) separates
identity from its execution body. How these principles should extend to an
operator-managed application service needs explicit maintainer agreement;
the existing remote-deployment model does not itself establish that product flow.

## Product motivation: an established Slack workflow

Slack provides a concrete product reference for this request. The last column describes the requested Buzz outcome, not a claim about current release support.

| Team requirement | Documented Slack / n8n reference | Requested Buzz experience |
| --- | --- | --- |
| One shared agent identity | Install an agent app in the workspace; users with access can discover and use it. [Slack guide](https://slack.com/help/articles/33076000248851-Work-with-AI-agents-in-Slack). | Register once for the community; authorized members find the same deployed identity in Agents and New message. |
| Private and team conversations | The same installed agent can receive DMs and participate in channels where it is added. See the Slack guide above. | Members can DM the agent or invite/mention it in permitted channels, according to explicit policy. |
| Multiple personal sessions with one agent | **New Chat** and history organize conversations; agent sessions are separate threads, including within DMs, with titles and lifecycle status. [Session model](https://docs.slack.dev/ai/agent-sessions/). | Each user starts and resumes separate private sessions with that same agent, without cloning it or mixing unrelated conversation context. |
| An external agent runtime such as n8n | Published n8n agents have a native Slack channel; reasoning, tools and memory remain in n8n. [n8n integration](https://docs.n8n.io/build/build-and-manage-agents.md#reach-agents-from-other-places). | An open, documented integration contract lets n8n and other runtimes provide the agent service, configured once server-side. |

For example: one company Purchasing agent; Alice has separate private sessions for Supplier A and Supplier B, Bob has his own sessions, and authorized colleagues can also mention that same agent in a team channel.

This is an adoption requirement for teams comparing collaboration platforms. Supporting this workflow would make Buzz more competitive for company and department use, while retaining its open, self-hostable model and freedom to choose the external runtime. The reference is the user outcome; the implementation should follow Buzz's own identity, policy and protocol design.

Availability still depends on the Slack plan and app/backend implementation; some agent features require a paid plan. Department-level authorization and memory boundaries still need explicit policy. This comparison does not claim Slack automatically supplies every requested control. [Slack feature requirements](https://docs.slack.dev/ai/developing-agents/).

## Shared software service boundary

The intended agents are shared application services, available to a whole company or to authorized departments/teams. Users interact with a stable software interface backed by a service such as n8n.

These agents expose only the business capabilities explicitly configured and authorized by the operator, such as querying a company system or executing an approved workflow/API operation. They must not expose a personal coding harness, a general-purpose host shell/terminal, arbitrary host filesystem access, or an employee's local credentials on either a user's computer or the server hosting the service.

An external backend can have its own internal agent execution engine. Using the agent in Buzz must grant only the declared application capabilities, without granting control of that engine's host computer or inheriting a human owner's computer authority. Registration and invocation must not require an employee's desktop coding harness.

This separates the requested service from a personal computer agent: the runtime/operator enforces the service's capability boundary, while Buzz enforces who may discover, invoke and administer it. Both boundaries must hold for authorized company/department users.

## Community management and access control

"Community-managed" describes administration and availability; it does not redefine cryptographic authorship. Reuse Buzz's existing identity, owner-attestation and admission mechanisms wherever appropriate, and agree how an authorized community operator represents and maintains this agent.

- Separate permission to **discover** an agent, **use** it (start a DM, invoke it or add it to a permitted conversation), **manage** its runtime/configuration, and **grant or revoke** access.
- Support access policies for the whole company/community, specific departments/teams (mapped to existing roles/groups where supported), and individual members. Joining the community does not automatically grant every agent or conversation permission.
- An authorized non-owner can use the agent without receiving its signing key, backend credentials or lifecycle controls.
- Enforce authorization at the trusted server boundary for registration, discovery and each invocation. Hiding a UI control is not access control. Recheck authorization before publishing delayed results.
- Keep agent-use permission separate from access to a conversation, its history, private runtime memory and external tools. An agent must not convey information from another user's DM or an unauthorized channel.
- Define revocation and administrative handover so this can be a durable workspace service rather than an integration dependent on one employee's desktop or provider login.

Reuse the existing authorization model where sufficient; discuss any necessary extension explicitly.

## Existing contracts and repository boundaries

- [NIP-OA](nips/NIP-OA.md) attests an independent agent key. It does not transfer
  authorship or grant channel membership. [NIP-AA](nips/NIP-AA.md) describes
  owner-derived relay access, separately from conversation admission.
- [Availability](agent-availability.md) is relay presence, not a deployment receipt
  or proof that the external backend can execute a request.
- [Remote agents](remote-agents.md) and the [ACP harness](../crates/buzz-acp/README.md)
  provide existing execution boundaries to evaluate. An ACP harness already
  handles Buzz subscriptions, author gates and conversation routing; reuse that
  responsibility if the ACP option is selected.
- New desktop/web work belongs in [block/buzz-app](https://github.com/block/buzz-app);
  mobile, relay and harness work belong here. Extend its
  [shared selection](https://github.com/block/buzz-app/blob/70a8aebdc9656bd808f791d91b283937ec37cec9/docs/agents.md#shared-agent-selection),
  preserving admission checks rather than rebuilding eligibility in each picker.
- Reuse signed events and authoritative policy/membership sources where sufficient.
  This RFC allocates no event kind and proposes no new relay HTTP route.

Source context: block/buzz `12f3d510011b1c57c1f51f9aaa25a307fa0bb883` and
block/buzz-app `70a8aebdc9656bd808f791d91b283937ec37cec9` (2026-11-07).
These are architectural starting points, not evidence that runtime/client
acceptance has passed in an existing release.

## Open runtime boundary: options to evaluate

The desired shape is independent of a transport choice:

```text
Desktop / Web / Mobile
          |
       Buzz relay
          |
  server-side integration boundary
          |
  externally hosted application agent
```

| Candidate | Existing pieces to reuse | Work still requiring agreement |
| --- | --- | --- |
| Existing `buzz-acp` plus ACP-to-HTTP adapter | Harness subscriptions, author gates, sessions and presence | Stable structured turn metadata, external-runtime invocation, signed result publication and community-wide discovery |
| Relay-connected gateway to an external runtime | Buzz event builders, authenticated event transport and signed profiles/policies | Scoped routing, invocation policy, session correlation, bounded recovery and client discovery |
| Existing hosted-provider extension model | Provider lifecycle and identity/admission concepts | Generic operator-controlled registration and eligible non-owner selection across clients |

ACP remains an option, not a required architecture. A runtime speaking ACP may
not need an HTTP adapter. MCP or another protocol should be considered for the
specific responsibility it covers; tool access alone is not a full agent
registration, conversation and authorization contract.

A publicly documented, versioned boundary should let independently implemented
runtimes or adapters interoperate. An HTTP request/response payload would be a
proposed adapter contract, not an existing ACP-over-HTTP standard. External
execution stays outside the relay's request handlers.

Existing workflow `call_webhook` can serve automation but does not establish
the directory, agent identity, permission or private-session guarantees here.
No unrelated shell, repository checkout or model credential is required merely
to forward a request. Extra tools require a declared capability and permission.

### Runtime contract items to agree

| Data | Purpose |
| --- | --- |
| Contract version and capabilities | Negotiate supported input, results, cancellation, approval/resume, streaming and asynchronous modes |
| Invocation ID | Correlate one logical turn across transport attempts |
| Session ID | Map the selected Buzz conversation/session to the external runtime |
| Community, agent and destination references | Bind the turn to its admitted origin |
| Authenticated caller and trigger references | Enforce invocation policy and correlate accepted input |
| Input blocks and bounded context | Supply authorized content without unrelated history |
| Deadline and response mode | Bound work and distinguish acceptance from completion |

Routing context comes from trusted integration state, not rendered message text.
A prompt cannot select the endpoint, credential, agent identity or destination.
If the chosen existing boundary lacks stable metadata, agree a narrow structured
extension rather than parsing authority out of a prompt.

The reference integration should call a supported production interface. For n8n,
[Message an Agent](https://docs.n8n.io/integrations/builtin/core-nodes/n8n-nodes-base.messageanagent.md)
is an available entry to a published agent from a production workflow, with session
support. That is a reference backend, not a protocol requirement. A Preview-only
call or successful echo webhook does not prove production, memory, approval or
long-running execution support.

### Completion and signed publication

A synchronous first slice may return one bounded final result. Long-running jobs
need an agreed asynchronous contract: durable acceptance, stable invocation ID,
authenticated completion, deadlines, and cancellation semantics. Declare
unsupported modes rather than pretending the backend implemented them.

A completion must match the stored community, agent, session and destination.
Publish only intended replies through existing Buzz builders and event transport,
signed under the agent identity. ACP text chunks are not automatically a chat
reply; private reasoning must not be published. Backend HTTP success is not a
relay publication acknowledgement.

Persist the exact signed reply/event ID before first submission. After an
ambiguous acknowledgement, resolve or retry that same event rather than signing
a new duplicate. Retain recoverable publication state and recheck current
destination admission before releasing delayed output.

## Community discovery and multiple private sessions

A one-time registration needs an authorized identity/policy, public display
metadata and an operator-private execution mapping. URLs, tokens and signing keys
do not belong in public agent profiles. Agree which trusted source lets eligible
members discover an agent administered by someone else.

Owner-only provider inventory, a shared template, or a bot in channel members is
insufficient. A card with an unavailable runtime is not proof of usable DMs.
Do not fabricate local custody or label a runtime as another provider.

Clients project the same exact public key and policy into Agents, New message,
profile Message and channel invitation/mention flows. Discovery does not grant
membership, invocation or management authority. Failed discovery must remain
distinguishable from an authoritative empty list.

A user can start a new private session with the same agent, identify it in history
and resume the intended one. Multiple sessions may be threads in a DM or another
agreed native surface; this does not require cloning the agent per task or user.
Map session state using community, agent and authorized conversation/session
identity. Keep private histories distinct; intentional cross-session memory
requires explicit policy and cannot widen access across people or departments.

Ordinary members receive only allowed conversation actions. Lifecycle controls
remain with authorized administrators/operators and reflect actual custody.
An adapter with a live relay socket and an unreachable backend cannot advertise
itself as healthy solely because the socket remains connected.

## Privacy and reliable recovery

- Keep agent signing custody in the trusted server-side publication path. The
  human owner's key never enters the backend, workflow export or runtime payload.
- Store exact endpoints and per-agent secret references under operator control.
  Reuse outbound HTTP/SSRF protections; message text cannot choose a URL or
  redirect. Private backend access needs explicit operator policy.
- Send bounded authorized context, not unrelated DMs, credentials, hidden reasoning
  or personal host resources. Buzz records and private backend memory have
  separate owners and access rules.
- Persist accepted asynchronous work and recovery state before acknowledging it.
  Duplicate input reuses the invocation ID. Ambiguous backend acceptance cannot
  safely be retried as a new turn; define backend idempotency and reconciliation.
- Bound queues, payloads, retries, deadlines and retention. Preserve per-session
  ordering and isolate concurrent users, departments and communities.
- Reject forged or mismatched callbacks and late results after revocation,
  cancellation, expiry or a superseding invocation. Report whether cancellation
  actually stopped execution; closing HTTP does not prove that it did.
- Do not promise exactly-once external side effects or turn an unknown result into
  success. Restart recovery must preserve session and publication correlation.

## Acceptance criteria

1. An authorized operator registers an externally hosted agent once, with no local configuration required for ordinary members.
2. Two authorized humans, including a non-owner, discover that same agent and start DMs on supported desktop, web and mobile clients.
3. They can invite/mention it in allowed channels; replies preserve the agent identity and conversation/thread references.
4. Exercise both a company-wide agent and a department-scoped agent. Authorized members can use the appropriate service; a member outside the allowed department cannot discover or invoke its restricted agent through either the UI or direct protocol requests. A member allowed to use an agent cannot gain management rights.
5. Access revocation takes effect at the trusted boundary, including delayed results. Administrative handover does not require re-creating an agent per user.
6. Responses continue with all desktop apps closed. Concurrent private conversations and communities cannot leak context or memory into one another.
7. Duplicate input, timeouts, ambiguous acceptance, restart and unsupported backend capabilities produce defined, truthful outcomes.
8. An n8n reference integration and an independent deterministic fixture demonstrate the same documented contract, with secrets omitted from exports.
9. Invocation grants only the declared application capabilities. Neither a prompt nor a direct protocol request can grant host shell/terminal access, arbitrary host filesystem access, personal coding-harness authority or host credentials.
10. Each of the two humans starts and resumes at least two independent private sessions with that same agent. Verify correct session routing and continuity, with no unintended cross-session or cross-user context sharing.
11. Reject forged registration, cross-community input, unauthenticated completion,
    changed destinations and attempts outside the registered capability boundary.

Implementation may be staged, but execution transport alone does not complete the discovery, authorization and cross-platform acceptance criteria.

## Decisions and staged contribution

1. Confirm product fit for shared application agents and the supported client
   versions before treating an adapter implementation as the agreed next step.
2. Agree the authenticated registration/directory authority, per-member invocation
   policy and administrative handover. Reuse identity and membership semantics.
3. Select the smallest suitable open runtime boundary and negotiate capabilities;
   agree where the reference integration and conformance fixture belong.
4. Implement missing discovery, selection and private-session behavior in the
   current desktop/web and mobile clients. Keep deprecated `desktop/` feature
   work out of scope.
5. Document one server deployment and a minimal n8n example after these boundaries
   are agreed. Demonstrate identical contract behavior with an independent fixture.

Each implementation needs its own real-boundary validation. Synchronous transport
alone does not complete cross-platform discovery, authorization or session parity.
Asynchronous work and approval/resume remain unsupported until declared and
validated. A server cannot inject missing UI into old clients.

Related: [external roster #3996](https://github.com/block/buzz/issues/3996),
[external identity registration #8012](https://github.com/block/buzz/issues/8012),
[hosted execution discussion #4174](https://github.com/block/buzz/issues/4174),
[non-owner invocation report #8684](https://github.com/block/buzz/issues/8684), and
[hosted DM report block/buzz-app#1090](https://github.com/block/buzz-app/issues/1090).
Historical reports need reproduction on the target release. These discussions
do not establish a current delivery commitment. [PR #3803](https://github.com/block/buzz/pull/3803)
targets the deprecated desktop and is not this proposal's implementation base.

