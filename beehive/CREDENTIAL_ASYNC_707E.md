# Cancellation-fenced credential access — implementation candidate, not OS readiness

## Boundary

`credentialHelperReader` owns one one-shot Node subprocess, stdin/stdout pipes only,
1 KiB response cap, 10-second default deadline (maximum 30 seconds), and refuses
concurrent reads. It sends only a public Beehive reference in the request. No
private argv, inherited provider environment, native error text, journals, daemon,
Worker termination assumption or secret cache. Cancellation/deadline sends SIGKILL
and waits for child `close`, not the boolean return from kill. The trusted child
has no subprocess creation. Blocking fixture uses Atomics.wait so its JS abort
handlers cannot run; tests check that the owned PID no longer exists on settlement.
This demonstrates process cancellation, **not** Keychain SecurityAgent dialog
cancellation or OS availability. An OS-owned dialog may survive the caller; actual
owner-present validation remains mandatory. Windows process semantics unvalidated.

Default native live reads refuse before constructing any Entry. An explicit
operator-approved helper reader can be composed into `CredentialBackend.readAsync`
after independent review and the owner-present OS step; it is not automatically
installed/enabled by host startup. Local deliberate provision/import/remove retain
the existing synchronous adapter. Native missing/null is distinct from thrown
locked/denied/unavailable; timeout says possibly prompt-blocked, not proven locked.

Host hydration and every pre-spawn reread use the async boundary. Stop cancellation
is received outside the serialized executor, aborts the credential read and waits
for cleanup before its queued completion. Close uses the same ownership boundary.
Late results are checked for cancellation, revision, placement, selected-next and
public-manifest changes; callers recheck after the prepare await before effects.
Startup cancellation cannot release host.lock before hydration cleanup completes.
Fresh key loading at Start remains; deliberately removed keys cannot come from a
hydrated-secret fallback. Explicit fixture backends can remain synchronous; native
backends cannot silently fall through to synchronous live reads.

## V3 local progress

- `addSlot` now dispatches v3 to store -> verify -> public-reference activation,
  reusing a local binding without sibling key copies. Genesis preserves standby
  assignment. Existing/public-only identity refuses duplicate creation.
- V3 `add-agent` uses a public-only overview, asks before generating a new agent,
  or accepts a public standby genesis and hidden matching-key input. V3 does not
  accept a plaintext private-key file. This is independent of the explicitly
  retained legacy diagnostic command paths.
- `reconcile-provision` explicitly resumes FIRST-provision failure only: same
  public setup, matching supplied key/genesis, exact inert journal, no active
  manifest. Existing matching orphan key is verified, never overwritten. Missing
  key is created only by this explicit matching-key action. Journal bytes remain;
  an active/public-only manifest refuses recovery and requires explicit import.

Filesystem + OS store are not a transaction. Added-agent failure can still leave
an inert orphan journal/key and currently refuses retry pending guided addition
reconciliation. V3 immutable binding add/retire/conversion and full local wizard
remain unfinished. No automatic plaintext migration/deletion, no owner key minted
or copied to the host, and no relay admission or Start during provisioning.

## Evidence / remaining

Workspace `WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/CREDENTIAL_ASYNC_707E/` preserves exact
focused and candidate gate output. Controlled helper and v3 host tests cover blocked
read + Stop/close + deliberately late result + actual PID absence, and the prior
same-batch admission suite still covers receive-order/revision/auth fences. V3
failure injection covers before-store and after-store first-provision recovery,
wrong identity, standby addition, retained deletion refusing recovery/recreation,
and byte-identical sibling journal. These are isolated fixtures, no OS operations.

Independent async semantic review and actual owner-present OS verification remain.
No new v3 local provisioning CLI process journey is claimed yet (API tests only).
Existing installed owner-TUI private-wire reply journey remains separate evidence.
Direct ordinary membership is approved; source contract is now available in
`WORK_LOGS/BEEHIVE_DESIGN_183FFAF0/DIRECT_MEMBER_EVIDENCE_AA0B3F26.md`; runtime
implementation remains. Private profile distribution, reconnect and new-wire Move
also remain. This is not the completed usable milestone or an install candidate.
