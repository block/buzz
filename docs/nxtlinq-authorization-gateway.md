# Nxtlinq Authorization Gateway

Buzz can launch an ACP runtime through `nxtlinq-authorization-gateway`. The
Gateway verifies the signed project policy before forwarding supported ACP and
Buzz Agent tool operations.

This integration deliberately separates operator-owned configuration from
per-Agent configuration:

- Buzz Settings owns one Gateway installation, one trusted-signers bundle, and
  one receipt root for the local Buzz installation.
- Each Agent chooses only its project workspace. Buzz derives a private receipt
  subdirectory from the Agent public key.
- The project owner keeps the signed Nxtlinq manifest in the project.
- The signing private key never enters the Agent or WebView. During explicit
  first-run initialization, Buzz invokes the reviewed Attest CLI's standard
  `init`; Attest generates the key pair and manifest. Buzz verifies the pair,
  relocates the private key into the operating system's secure credential
  store, and installs only public project state. If that store is unavailable,
  Buzz uses an owner-only file under its app-data directory, outside the Agent
  workspace, and reports the fallback. For the local deployment MVP, that same
  explicit owner action copies the verified public key into a Buzz-owned trust
  store outside the workspace and selects it automatically.

This integration pins the reviewed `@nxtlinq/authorization-gateway` 0.3.0
package so Buzz can delegate setup verification to the Gateway's `--check`
command without silently adopting a newer, unreviewed authorization boundary.

## Current deployment model

The current integration is a **local deployment MVP**. Buzz Desktop, the
Gateway, the ACP runtime, the trust store, and the protected workspace are
expected to be available on the same execution host. Paths stored by Buzz,
including the trusted-signers path, are paths on that host; they are not URLs
or references to files that remain on a separate operator computer.

Two distinctions are important:

- The project owner and the Agent are cryptographically separated. The signing
  private key stays in Buzz-controlled native secure storage, and the Agent
  cannot replace its wrapper or grant itself trust through Agent environment
  variables.
- The local human who can edit Buzz's Nxtlinq Settings is currently also the
  deployment operator. That person can select a different trust store or
  receipt root. This MVP therefore protects the authorization boundary from
  the Agent, but does not enforce a hard administrative separation between an
  end user and an operator who controls the same Buzz installation.

For a single-user development machine this is intentional and explicit. A
managed installation where an administrator provisions trust and end users
cannot alter it is deferred; see **Managed operator/end-user separation**.

## Prepare and sign a project

The project owner prepares the project before an Agent can enable the Gateway:

1. Initialize the project with `@nxtlinq/attest` 3.x. Buzz's review dialog can
   run standard Attest init from zero, then move the generated private key into
   native secure storage. Existing identities can still be initialized manually with
   `nxtlinq-attest init --public-key <external-public-key> --key-id <key-id>`;
   such externally managed keys are not automatically imported into Buzz.
2. Create every file that should be covered by artifact integrity, including
   ignored local files such as `.env`.
3. Edit `nxtlinq/agent.manifest.json` with the
   `nxtlinq-authorization-gateway` audience and structured capabilities.
4. Run `nxtlinq-attest sign` after the final project changes.
5. Give the deployment operator the signer `keyId` and public key, never the
   private key.

`.gitignore` controls Git only; it does not exclude a file from the Attest
artifact. Adding or changing an included file after signing causes the Gateway
to fail closed with an artifact-integrity error until the project owner signs
again. For production, keep the private signing key outside the Agent-writable
project and use `nxtlinq-attest sign --private-key <owner-controlled-path>`.

The published Gateway package includes a clean project walkthrough at
`examples/buzz/README.md`. It starts without generated keys, signatures,
environment files, trust state, or receipts.

## Open setup from Buzz or an Agent conversation

Before signing or enabling authorization, the signing private key must be
outside every directory the Agent can access. Buzz-assisted initialization
enforces this by using native secure storage. If a manual default
`nxtlinq-attest init` created `nxtlinq/private.key`, migrate to a public-key-only
project initialization before using this flow. Buzz refuses conversational
setup while that standard key path exists, but it cannot discover arbitrary
copies elsewhere in the workspace.

The owner can open the complete flow without an Agent conversation: open the
Agent card, choose **Edit**, expand **Advanced**, and select **Set up Nxtlinq**.
Buzz starts with a conservative read-only policy; the owner can edit it beside
the current manifest before approving it. If the project already has a
manifest, Buzz adopts its current policy instead of replacing it with the
default. The editor exposes requested metadata and capabilities, while Buzz
keeps the inert scope, Gateway audience, sensitive exclusions, and required
session-only MCP connection in a separate locked-safeguards section. Those
fields are composed into the proposed manifest and cannot be removed through
the editor. Policy edits are validated automatically and refresh the
side-by-side manifest diff; **Reset to safe baseline** only replaces the
uncommitted editor draft. It does not write the manifest. Apply or activation
remains disabled until the owner explicitly acknowledges the latest
current-versus-proposed diff. Editing the policy again invalidates that
acknowledgement. The project is selected and confirmed only in the **Project** step;
Agent Edit does not expose a second workspace control. After authorization is
enabled, path changes go back through **Review policy**, which keeps the signed
project, Gateway wrapper, and ACP working directory aligned.

For a draft opened from an Agent conversation, **Regenerate proposal** sends
the latest valid permission proposal plus optional owner guidance back to that
same Agent in the originating channel. The Agent must submit a new structured
review request; it does not mutate the open draft, apply the manifest, or gain
access to signing material. The current proposal and guidance are sufficient
inputs: reading project files is optional, and a denial from filesystem, MCP,
terminal, or a help lookup must not prevent the Agent from invoking the trusted
structured setup tool in the same turn. The Agent never runs the setup CLI or
its help command through shell. The old Apply/Activate actions are disabled while
Buzz waits. When the replacement request arrives from that same owned Agent for
the same project, Buzz updates the Policy step and diff in place, preserving the
confirmed workspace and local trust while requiring acknowledgement of the new
diff. A response that changes the project root returns to the Project step for
explicit confirmation. Direct UI setup keeps the deterministic safe baseline
and does not silently choose a collaboration channel on the owner's behalf.

Alternatively, an owned managed Agent can help design the policy without
receiving authority to install software, write files, initialize keys, or sign
the manifest. Ask the Agent to configure Nxtlinq for a specific absolute
project path and describe the work it needs to perform. The Agent preserves an owner-supplied path rather
than substituting its shell, MCP working directory, configured workspace, or
`~/.buzz-dev/REPOS`. If the request provides no absolute path, it asks for the
exact project folder before submitting anything. For an explicit setup request,
the Agent inspects only ordinary project documentation/source already permitted
by its current policy and submits an encrypted owner-review request in the same
turn; it should not stop at a prose proposal or ask for a second confirmation
because the Desktop review is the confirmation boundary. An already protected
Agent may be denied the exact read, terminal, or MCP operation needed for newly
requested work. That is expected: the denial identifies the capability to
propose and does not prevent the trusted setup control plane from opening
review. The Agent must not retry a bypass or ask for a narrower target solely
because current authorization is insufficient. For a generic request it falls
back to the conservative baseline, and the owner can edit the proposal beside
the existing manifest before apply. The request contains:

- the current channel and project root;
- policy-only fields (`name`, `version`, `scope`, `aud`, `capabilities`, and
  optional `exp`);
- a plain-language explanation of every requested path, command, and MCP
  connection.

The absolute project root and manifest patterns deliberately use different
representations. `owner_project_root` is an absolute host path used to locate
the initialized project, while every filesystem `include` and `exclude` is a
project-relative glob. Buzz normalizes an Agent draft that accidentally prefixes
the exact reviewed project root, rejects absolute paths outside that root, and
adds defense-in-depth exclusions for environment files, npm/netrc/PyPI
credentials, Git and Nxtlinq metadata, AWS/Docker/SSH credentials, and key
files. Desktop and the bundled CLI
then validate the normalized shape again before presenting or applying it.

Conversational drafts use exactly the inert compatibility marker
`scope: ["demo:structured-capabilities"]`; permissions live only in structured
capabilities. The safe baseline is a narrow `filesystem:read` plus
`mcp:connect` for `buzz-dev-mcp`. The latter permits session setup, not tool
invocation. Filesystem write, terminal execution, and `mcp:invoke` are omitted
unless the described future work needs them. Terminal entries are exact raw
command strings. Their environment constraint always includes `PATH` and names
every additional variable made visible to the command; values never enter the
manifest or receipt. The protected shell clears its inherited environment and
injects only those authorized names. Filesystem exclusions do not constrain an
allowed shell command. Host identity variables (`BUZZ_PRIVATE_KEY`,
`NOSTR_PRIVATE_KEY`, and `BUZZ_AUTH_TAG`) are never eligible for protected shell
injection. Each `mcp:invoke` capability names exactly one server so
multiple server/tool arrays cannot accidentally form a broader cross-product.
Conversational drafts reject `approvalRequired: true` until Buzz has a matching
interactive approval flow.

If the Agent only describes a policy and no review dialog appears, no draft was
submitted. A previous `outside_scope` denial is not a valid reason to withhold
the draft. Restart the Agent after updating Buzz so version 12 of its managed
`buzz-cli` skill is refreshed, then repeat the explicit setup request.

Buzz accepts requests only from an owned Agent that shares the claimed source
channel with the owner. Missing Attest initialization is not a draft-submission
blocker; Desktop detects it and presents the explicit initialization ceremony.
For an initialized project, Desktop reads the manifest itself, preserves
signer/public-key and integrity fields, validates the supported capability
shape, and displays the exact diff. Applying a diff is bound to the SHA-256 of
the reviewed file, so an intervening manifest change forces a new review.

The normal review is a progressive three-step ceremony: **Project**,
**Policy**, and **Activate**. Attest initialization, when needed, is performed
inside **Project**. **Local trust** appears as a separate step only when Buzz
does not yet have a saved trust and receipt configuration. The dialog renders
only the active step, and manifest or signing controls are not shown while the
owner is still choosing a workspace. Back revisits an earlier review step
without reversing native work the owner already approved. The flow covers Gateway installation, Attest
initialization when needed, local trusted-signers and receipt paths, manifest
application, Gateway verification, and enabling the wrapper for the requesting
Agent. It shows the proposed target path and permits the owner to browse to a
different project. If the project is uninitialized, the owner first uses
**Use as Agent workspace**, enters a signer key ID, and chooses **Initialize
securely**. That action stops and locks every local runtime pair
for that Agent and invokes the reviewed Attest CLI's standard `init` in a
protected mode-0700 staging directory beside the project. Attest generates the
private key, public key, and initial manifest; no existing public key is needed.
Buzz verifies that the generated keys match, assigns the owner's signer key ID,
stores the private key in the operating system's secure credential store, and
removes it from staging. If the keyring is unavailable, Buzz writes an
owner-only mode-0600 fallback below its app-data directory and reports that
storage class. Buzz then atomically installs only the public key and manifest
into the project. Only the storage class, public fingerprint, and non-secret
Buzz-owned trust-store path return to the WebView; private bytes and paths do
not. Buzz writes a fingerprint-named `trusted-signers.json` under its protected
app-data directory. The file embeds the public key Buzz just generated and
verified; it never points back to the Agent-writable project `public.key`.
Buzz saves that path as the active local trust store. Receiving the conversation
request alone changes nothing; enrollment occurs only when the owner explicitly
approves initialization while the Agent is stopped.

Settings also offers **Reinstall Gateway** to repair the exact reviewed package
version and **Uninstall Gateway** for clean-install testing. Uninstall preserves
operator settings, receipts, manifests, and signatures. Buzz disables it while
any managed Agent still references the Nxtlinq wrapper, and the native command
rechecks the authoritative Agent store before removing the managed npm packages.

A manifest edit invalidates its old signature; an initialized project may also
start without a valid signature. When readiness reports either state, Buzz
offers **Sign manifest securely** as a separate owner action. Buzz retrieves
the matching key from native secure storage, then stops every local runtime pair
for that Agent and holds the runtime transition through signing and
verification. The reviewed, Buzz-managed `@nxtlinq/attest@3.0.0` CLI signs the
exact SHA-256 revision approved in the dialog, then verifies it against the
saved trusted-signers bundle. Any intervening manifest edit, policy mismatch,
missing or mismatched managed key, untrusted signer, symlink, unsafe fallback
file mode, or tool version drift fails closed.

Private-key bytes and fallback paths remain inside the native backend and are
not returned to the WebView, sent through the observer channel, stored in Buzz
settings, or included in errors. For CLI compatibility, Buzz materializes the
key in an owner-only temporary directory only for the signing invocation and
deletes it immediately afterward. Buzz intentionally has no private-key file
picker or WebView drag-and-drop surface.

After signing, choose **Recheck & enable Agent**. This saves the verified
wrapper configuration. The Agent remains stopped until the owner explicitly
restarts it. Buzz never asks the Agent for the private key and never includes it
in the encrypted draft. Buzz never runs Attest's default initialization in the
Agent workspace directly, because version 3.0.0 initially creates
`private.key`. Instead it runs that standard initialization in protected
staging while the Agent is stopped, moves the Attest-generated private key into
native secure storage (or the protected app-data fallback), removes it from the
staged project state, and atomically installs only public material.

The Gateway deliberately starts the downstream ACP runtime with a clean
environment. Buzz therefore supplies an explicit name-only allowlist covering
its effective provider credentials, model/endpoint overrides, and Agent tuning
variables, including values inherited from global, persona, or build settings.
The values remain process environment state and are not written into the
manifest or review request. Downstream stderr is forwarded into the managed
Agent log so a missing runtime setting is reported directly instead of being
masked as a generic ACP initialization failure.

This flow does not retroactively sandbox an already-running, unprotected Agent.
Moving signing material and other secrets out of its workspace before the
conversation is an operator responsibility.

## Install and configure

For the usual per-Agent setup, open the Agent card, choose **Edit → Advanced →
Set up Nxtlinq**, and complete the owner review. That flow installs the Gateway
if needed, initializes Attest securely, creates local trust, applies and signs
the reviewed policy, and enables the Agent wrapper. The conversational route
opens the same owner-controlled review with an Agent-suggested policy.

The Settings card is the advanced installation and shared local trust surface:

1. Open **Settings → Agents → Nxtlinq authorization**.
2. Select **Install Gateway**. Buzz installs and verifies the reviewed Gateway
   0.3.0 package in its private managed Node tools directory. Use **Reinstall
   Gateway** to repair a missing or mismatched managed installation.
3. For an existing or externally managed identity, select the
   operator-provided `trusted-signers.json` file. Buzz-assisted initialization
   creates and selects a protected local trust store automatically.
4. Keep the default receipt root or choose another owner-controlled directory.
5. Save the Nxtlinq settings.

Buzz validates that the trust store is an existing JSON file, creates the
receipt root when necessary, and restricts its local configuration and receipt
directories to the current OS user on Unix systems.

The signer key normally originates with the project owner. For local
development, `nxtlinq-attest init` generates `nxtlinq/private.key` and
`nxtlinq/public.key`; an external signing authority may instead provide its
public key to `attest init --public-key ... --key-id ...`.

For an existing or externally managed identity, the trust store remains a
separate enrollment decision. Whoever controls the Buzz deployment verifies
the project owner's signer public key through an appropriate trusted channel,
then records that key and its `keyId` in an external `trusted-signers.json`.
Buzz-assisted initialization is narrower: while the Agent is stopped, Buzz
generates and verifies a fresh pair through reviewed Attest, protects the
private key, and embeds the verified public key in Buzz-owned app data during
the same explicit owner action. Buzz must never automatically trust a
repository-local key discovered outside that ceremony. The trust store must
never contain the signing private key.

For example, after independently verifying the project owner's public key, the
deployment operator can enroll it by path:

```json
{
  "trustedSigners": [
    {
      "keyId": "project-owner-2026",
      "publicKeyPath": "./keys/project-owner-public.pem"
    }
  ]
}
```

## Enable an Agent

1. Create the Agent with its real runtime, such as Codex or Goose.
2. In the Agents list, open the Agent card, select **Edit**, and expand
   **Advanced**. The similarly named profile/persona editor does not contain
   runtime launch settings.
3. In **Nxtlinq authorization**, select **Set up Nxtlinq**.
4. In **Project**, browse to the protected project and confirm **Use as Agent
   workspace**. Buzz uses the same path for the Gateway `--project` root and ACP
   session working directory.
5. Review the automatically updated side-by-side diff, acknowledge that exact
   revision, and apply the policy. Sign when required, then recheck and enable
   the Agent.

For an Agent that already has Nxtlinq enabled, changing its protected project
or the shared operator paths invalidates the prior readiness result. Reopen
**Review policy**; successful activation rebuilds the stored wrapper arguments
from that exact checked configuration. The normal update flow is:

```text
change project or shared operator paths
→ Review policy
→ Recheck & enable Agent
→ restart the Agent when requested
```

Disabling and re-enabling the preset is not required. Readiness is a
point-in-time configuration check; the Gateway independently verifies the
project, signature, trust store, and receipt destination again at process and
session startup.

Buzz builds the wrapper argv rather than asking the end user to enter it. Its
effective shape is:

```text
nxtlinq-authorization-gateway \
  --adapter acp \
  --project <agent-workspace> \
  --trust-store <global-trusted-signers.json> \
  --receipt-dir <global-receipt-root>/<agent-pubkey> \
  --mode acp-enforce \
  --pass-env BUZZ_AGENT_NXTLINQ_PERMISSION_BRIDGE \
  --pass-env BUZZ_AGENT_REQUIRE_REPLY \
  -- <selected-acp-runtime> <runtime-args...>
```

The preset also enables the downstream Buzz Agent permission bridge and reply
delivery. Additional credentials required by the selected runtime are passed
by name; the Gateway does not inherit every ambient Buzz environment variable.

`BUZZ_ACP_TRUST_NXTLINQ_GATEWAY` is not an end-user setting. Buzz removes it
from configurable Agent environment variables and derives it at process spawn
only when the wrapper carries the readiness result's verification binding and
still resolves to Buzz's managed Gateway 0.3.0 package with the same executable
SHA-256. A same-name executable, PATH or in-place executable substitution,
changed package version, or unverified wrapper record does not receive the
flag. This prevents an Agent configuration from granting itself the Gateway
trust path.

## Local Gateway development

The Settings installer uses the published npm package. To test an unpublished
local Gateway build on macOS, install it into Buzz's managed Node tool prefix,
then launch Buzz normally:

```sh
cd /absolute/path/to/nxtlinq-authorization-gateway
npm run build
npm install -g . --prefix "$HOME/Library/Application Support/Buzz/node-tools"

cd /absolute/path/to/buzz
just desktop-standalone
```

Do not prepend a local Gateway repository to `PATH` when testing the published
package. That override makes Buzz execute the local checkout instead of the
version installed by **Install Gateway**. Once the Gateway is present in
Buzz's managed tool directory, launch Buzz normally with
`just desktop-standalone`.

## Policy denials

The Gateway reports a deterministic policy denial as JSON-RPC error `-32041`.
Buzz does not retry that same policy decision. The Agent should still publish a
normal-language response describing which operation was denied.

When a prompt is denied unexpectedly, inspect the matching receipt's `reason`
and `capability`. A denial during `session:create` commonly means a Buzz-provided
MCP server is absent from the manifest. For development with `buzz-dev-mcp`, the
signed manifest must include:

```json
{
  "type": "mcp:connect",
  "servers": ["buzz-dev-mcp"]
}
```

Re-sign the manifest after changing its capabilities.

## Security boundary and current scope

- Buzz validates only the narrow policy-draft shape needed for safe review; it
  does not decide whether an operation is authorized. Signature verification,
  policy evaluation, receipts, and the `--check` contract belong to the Gateway.
- Buzz owns safe installation/discovery, global public trust configuration,
  structured wrapper construction, launch-time trust derivation, and the host
  integrations that route supported operations through the Gateway.
- The Buzz Agent bridge protects its file tools and maps shell execution to a
  separate `terminal:execute` decision. A denied `read_file` therefore cannot
  be bypassed with `cat` through that shell tool.
- The bridge is fail-closed for every non-exempt MCP call. Only state-only
  lifecycle tools and the structured reply/setup control plane on the pinned
  Buzz MCP server bypass per-operation checks. Unknown and third-party tools
  require an explicit `mcp:invoke` server/tool grant; a same-named third-party
  tool cannot inherit Buzz's filesystem or terminal mapping.
- When the bridge is active, `_Stop` and `_PostCompact` hooks are pinned to the
  bundled `buzz-dev-mcp` server even if the ordinary runtime hook setting uses
  a wildcard. Third-party servers cannot use lifecycle hooks as an unreceipted
  invocation path.
- Local `view_image` reads are authorized against its real `source` path;
  remote image fetches require `mcp:invoke`. `str_replace` requires both read
  and write grants for the same canonical file. Buzz replaces the MCP argument
  with that canonical path before execution, preventing the original symlink
  argument from being resolved independently a second time. This reduces path
  drift but is not an inode-pinned OS sandbox or a complete TOCTOU defense.
- Project/global hint discovery and the in-process `load_skill` filesystem
  reader are disabled while the Nxtlinq bridge is active. A future version may
  restore them only after routing each resolved file through the same receipt-
  bearing filesystem authorization path.
- Conversational replies use a dedicated structured message tool so a denied
  terminal policy does not prevent the Agent from reporting the denial. The
  host binds that tool (and the setup-draft tool) to the active channel; the
  model cannot redirect either control-plane operation to another channel.
- This is not a complete OS sandbox. A downstream runtime or third-party MCP
  server with an independent filesystem, process, or network channel remains
  outside this Gateway path unless that channel is separately brokered.
- Full binary provenance/update lifecycle, remote trust-bundle distribution,
  Guardian policy composition, and a receipts timeline are intentionally
  deferred from this installation MVP.

## Managed operator/end-user separation (deferred)

A future managed deployment should make the deployment operator a real
administrative role instead of assuming that the local Buzz user performs both
roles. This is a follow-up feature, not a completion condition for the current
integration.

The managed design should:

- provision an operator-approved trust bundle onto every machine that actually
  runs the Gateway;
- treat `trusted-signers.json` and its referenced public keys as one versioned,
  atomically deployed bundle;
- lock the trust-store and receipt-root configuration so ordinary end users
  can view status but cannot replace either path;
- distinguish an explicit **Local developer** mode from an administrator-owned
  **Managed** mode;
- configure trust on the remote execution host when the Agent runs remotely,
  rather than storing a Desktop-local path that the remote host cannot resolve;
- support signer rotation and revocation without editing every Agent; and
- expose bundle source, version, validation state, and last update for audit and
  troubleshooting.

The minimum acceptance criteria are that an end user cannot expand the trusted
signer set, the Agent cannot write operator configuration or trust material,
and every wrapper launch resolves only the bundle provisioned for its execution
host. Local developer mode may remain available, but it must be visibly marked
as a mode in which the local user is also the operator.

See the Gateway's `docs/acp-host-integration.md` for its policy, receipt, and
conformance contract.
