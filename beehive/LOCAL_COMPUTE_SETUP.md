# Connect an already provisioned local compute gateway

This is connection setup, **not mesh/backend provisioning**. Beehive neither
installs nor starts a compute service, opens a public listener, nor manages its
credentials. Use a gateway you have separately provisioned as the host service
operator, with a real required bearer token and OpenAI Chat Completions support.

1. On the host, arrange for that gateway to listen on literal `127.0.0.1` or
   `[::1]`. Its API base must be `/v1`. Keep its service configuration local.
2. Store its actual required bearer token in an absolute, canonical owner-only
   (0600) file owned by the Beehive service user. Do not put credentials in a URL,
   argv, profile, remote TUI or shared inventory.
3. In the existing local setup wizard select Buzz Agent provider setup, the
   installed buzz-agent executable, provider `openai-compat`, that key file,
   `http://127.0.0.1:PORT/v1`, wire `chat`, and exact served wire model IDs.
   Existing `local-setup` → `add-buzz-provider` also creates a new immutable binding;
   do not mutate a binding used by an existing run. HTTPS services remain supported.
4. Complete normal conversation setup with separate conversation relay and
   installed buzz-acp/Buzz CLI. The model HTTP URL is **not** a management or
   conversation relay. Existing workspace, identity and profile controls apply.
5. Select the advertised binding/model in a public named launch configuration.
   Save changes selected-next only. Explicit Start/Restart gates prompts on the
   exact session model/profile. Failed preflight preserves the previous actual
   run. Endpoint availability or an approved model list alone is not readiness.

## Source contract and unsupported direct mesh mode

At Buzz source `051c3a270be9c73da9ab06700bcab7d5552fceaa`,
`desktop/src-tauri/src/managed_agents/relay_mesh.rs` derives OpenAI-compatible
Chat Completions at `http://127.0.0.1:9337/v1`, maps Desktop `auto` to wire model
`mesh`, and inserts the **placeholder** `buzz-mesh-local`. The frontend is not
thereby authenticated. `managed_agents/runtime.rs:778–789` removes ambient
`OPENAI_API_KEY`. `crates/buzz-agent/src/config.rs:631–691` requires nonempty
`OPENAI_COMPAT_API_KEY`; `llm.rs` appends `/chat/completions` and uses bearer auth.
Beehive retains this closed provider environment and does not insert a placeholder
or fall back to ambient credentials. **Direct unauthenticated Desktop mesh is not
supported by this slice.** Native optional-auth support would require separately
authorized consumer changes; do not make a fake key to appear configured.

Use exact wire IDs, not the Desktop `auto` alias. `mesh` may route/degrade among
workers, so exact wire-model confirmation does not attest physical model/committee
membership. No mesh-specific long deadline, generation defaults or capacity promise
is imported. `commands/agent_providers.rs` describes discovered `buzz-backend-*`
JSON info invocations for compute deployment, not another LLM provider type; those
backend operations are not implemented here.

## Evidence scope

The reused installed normal OpenAI-compatible journey drives the actual local
wizard, host CLI subprocess and remote TUI; real installed buzz-acp and Buzz CLI
publish four verified same-agent signed replies. A source-shaped TypeScript ACP
consumer calls a bearer-protected loopback Chat Completions fixture. It checks
exact model/native profile, fresh Restart, selected-next versus actual, sibling and
history preservation, HTTP wrong-model/malformed-protocol/unreachable failures,
missing required credentials and no private inputs in inventory. This is not real
Buzz Agent inference, a deployed mesh network or production provider attestation.
