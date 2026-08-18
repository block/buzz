# buzz-ifc-broker

`buzz-ifc-broker` exposes the shared `buzz-ifc` policy over bounded JSON-RPC
2.0 on stdin/stdout. It exists for trusted agent harnesses that cannot link the
Rust crate directly, such as a Go kgoose integration.

The broker owns process-level IFC state. The caller supplies facts only after
it has verified the triggering event and authoritative Buzz membership. The
caller does not choose an audience or context: `buzz-ifc` derives both from the
verified conversation kind, roster, requesters, executing agent, and owner.

The agent runtime must not receive raw Buzz credentials or another route around
the trusted adapter. Despite its executable name, this is the policy process
inside the agent gateway, not the complete product broker. It does not launch,
sandbox, or terminate workers and cannot force its caller to obey a decision.

Every derived domain includes a compartment profile:

- `shared_public` may reuse the realm's normal public worker and public state.
- `domain_confined` requires a worker, writable state, and output paths dedicated
  to that exact restricted or owner-private domain.

The trusted adapter implements that placement. A local sandbox, container, or VM
supplies the final confinement boundary for `domain_confined` work.

Run in compatibility mode:

```bash
buzz-ifc-broker --mode audit
```

Run when the trusted adapter will treat denied decisions as blocking:

```bash
buzz-ifc-broker --mode enforce
```

Each request and response is one JSON line. Start by binding a concrete worker
to the domain derived from verified invocation facts:

```json
{"jsonrpc":"2.0","id":1,"method":"worker/enter","params":{"worker_id":"kgoose-session-42","invocation":{"realm_url":"wss://buzz.example","channel_id":"00000000-0000-0000-0000-000000000001","conversation_kind":"restricted","epoch":"membership:<signed-event-id>","members":["<agent-pubkey>","<alice-pubkey>","<bob-pubkey>"],"executing_agent":"<agent-pubkey>","requesters":["<alice-pubkey>"],"owner":"<alice-pubkey>","bot_capabilities":["buzz.read.current","buzz.publish.current","email.read"],"conversation_capabilities":["buzz.read.current","buzz.publish.current"]}}}
```

The result includes the opaque `domain_id`; `details.replace_worker` tells the
adapter when the named process already contains another domain. The adapter
must retire that process before delivering the new request.

Call `worker/observe` before labeled data enters the process and `worker/call`
before invoking a mediated operation. If audit mode proceeds after a denied
call, its result must still be reported with `worker/observe`. Before Buzz signs
or sends a response, call `worker/publish` with the exact content digest and the
actual destination. Without a verified declassification grant, output is bound
to the source context. The current protocol intentionally exposes no
declassification method.

Supported methods:

- `broker/info`
- `domain/derive`
- `worker/enter`
- `worker/observe`
- `worker/call`
- `worker/publish`
- `worker/retire`

Logs are written to stderr. Stdout contains protocol frames only.

The broker connection is part of the worker lifecycle. `worker/retire` removes
policy state only after the adapter has terminated the corresponding process.
If the broker exits or the connection is replaced, the adapter must terminate
all workers tracked by that broker. A fresh broker has no evidence that an
existing process is safe to reuse.

`tests/fixtures/domain-golden.json` is the language-neutral conformance vector.
Adapters should replay it in their own test suite rather than duplicating the
domain hashing algorithm.
