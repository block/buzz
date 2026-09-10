# Private host catalog continuation (implementation checkpoint, not deployable)

The existing owner TUI now accepts `tui <catalog-file> <relay>` with an explicitly
entered existing owner signer. `catalog <new-file> <registration-files...>` retains
verified public registrations. Catalog validation binds the signature to owner,
host PUBLIC KEY, relay, expiration and label; duplicate labels are allowed but are
never routes. The new transport uses host public keys as Message.host. Historical
label routes are not silently migrated.

The owner management journal remains the single intent/receipt owner. Its private
Nostr adapter unwraps and verifies reports against the retained catalog before
inventory or receipt processing. Local encrypted intent envelopes are not broadcast:
their exact inner operation ID/body is rewrapped privately to one addressed host.
Relay ACK is never a terminal result. Expired registrations and unregistered host
responses fail closed; stale or absent inventory is UNKNOWN, not stopped.

The existing host executor accepts an internal transport dependency. The private
adapter verifies registration, host key, expected owner and command host; executor
startup also binds that transport to retained host/agent authority. Inventory and
receipts go only to the owner. The existing per-agent assignment, journal,
fingerprint, revision and cancellation checks still govern execution. Registration
alone does not initialize agent assignment or authorize an unprovisioned agent.
Move, host-host grants and private profile distribution remain refused. No new app,
registry daemon, HTTP service, production CLI test flag or OA host credential.

## Executable evidence

`test/catalog-tui.test.ts` drives the actual CLI/TUI subprocess, existing host
executor and external fixture runner. It observes verified private inventory,
registered-but-unreachable second host, Start/running/Stop/stopped, same-agent
identity and revision 2. A second independently admitted host receives no inventory
or commands; forged host inventory and unauthorized Start do not change state.
An exact retained Start is freshly wrapped after Stop and the journal stays
byte-identical; owner reconcile/reopen history cannot resurrect it.

The fixture uses independent direct-membership identities, signed NIP-42 AUTH,
1059 signatures/timestamps and authenticated-recipient p-gated reads. It is a narrow
executable model, NOT a real Rust relay or proof that production membership limits
a host to management operations. AUTH uses the source ±60 second window. Tests
inject only the typed admission dependency using an internal Node module loader;
normal CLI input/JSON cannot approve a connection. Production admission always
refuses before dialing. Tests do not sign as a real user or call a provider.

## Mandatory OS key storage — partial implementation, exact gate

Source report: workspace `KEY_STORAGE_SOURCE_2D3E2699.md`, pinned Buzz `051c3a270`:
Desktop uses Rust keyring 3.6.3 (macOS native Keychain generic password, Linux Secret
Service, Windows Credential Manager), a service-scoped JSON blob, cross-process
mutation lock and backend read-back verification. Beehive has no vetted TS native
bridge for those primitives. We do NOT access Buzz keychain items or spawn a tool
that might prompt. No OS integration or OS validation is claimed.

`credential-store.ts` is the minimum create/read/remove/reference boundary with
read-back and absence verification, distinct missing vs unavailable failures and
no implicit file fallback. Its production backend refuses with actionable bridge
requirements. Host identity schema v3 persists only a Beehive public credential
reference, pairing and registration. Host keys are created through the backend
before public metadata; v2 plaintext/OA files fail closed without migration.
Missing/removed keys never regenerate. A metadata-write failure may leave a retained
credential needing explicit recovery; no automatic deletion/recreation is attempted.
The old `identity` CLI command no longer creates a parallel controller key.

Tests explicitly inject in-memory storage or fresh fixture-only file storage across
pairing subprocesses. These are NOT OS keychain tests or a production fallback.
Production host creation is currently refused until a reviewed native bridge and
separate isolated-namespace validation authority exist.

**Still unfinished:** legacy diagnostic setup/slot APIs and CLI paths retain their
historical plaintext owner/agent storage. They are not a compliant production path;
do not use them with normal user keys. Converting that manifest, import/removal and
host loaders to public references is required before any installation or release.
The new private host adapter currently proves lifecycle with an explicitly isolated
legacy fixture installation, not a production owner-public-only agent installation.

## Review and remaining work

Consumed the independent a8 source report `REAL_HOST_SECURITY_A8E2424E.md`. Its
recommendation to use OA is superseded by the explicit owner requirement: AUTH
ViaOwner materializes an agent-owner relationship and is prohibited for hosts.
The report confirms that ordinary membership is broader than 1059-only. No deployment
permission is inferred from it. This delta removes caller label-map trust from the
new owner client and binds lifecycle dedup to the existing journals; it is not an
independent security review of the delta.

Remaining implementation: OS-backed host/agent/owner key lifecycle; agent public
installation schema and explicit placement signatures without owner-secret custody;
actual installed Buzz runtime/TS provider signed reply via this transport; private
Restart acceptance, host reconnect, profile distribution and retained-source-authority
Move integration. Current production gate must stay closed throughout. No machine
installations, release pin, mesh branch or production policy changed.
