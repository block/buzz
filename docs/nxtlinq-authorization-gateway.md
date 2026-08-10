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
- The signing private key is never imported into Buzz and must remain with the
  project owner or designated signing authority. The deployment operator only
  enrolls the corresponding public key.

This integration requires `@nxtlinq/authorization-gateway` 0.3.0 or newer so
Buzz can delegate setup verification to the Gateway's `--check` command.

## Prepare and sign a project

The project owner prepares the project before an Agent can enable the Gateway:

1. Install `@nxtlinq/attest` 3.x and run `nxtlinq-attest init` from the project
   root.
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

## Install and configure

1. Open **Settings → Agents → Nxtlinq authorization**.
2. Select **Install Gateway**. Use **Reinstall Gateway** when testing a newer
   Gateway build or replacing a version older than 0.3.0.
3. Select the operator-provided `trusted-signers.json` file.
4. Keep the default receipt root or choose another owner-controlled directory.
5. Save the Nxtlinq settings.

Buzz validates that the trust store is an existing JSON file, creates the
receipt root when necessary, and restricts its local configuration and receipt
directories to the current OS user on Unix systems.

The signer key normally originates with the project owner. For local
development, `nxtlinq-attest init` generates `nxtlinq/private.key` and
`nxtlinq/public.key`; an external signing authority may instead provide its
public key to `attest init --public-key ... --key-id ...`.

The trust store is a separate enrollment decision. Whoever controls the Buzz
deployment verifies the project owner's signer public key through an
appropriate trusted channel, then records that key and its `keyId` in an
external `trusted-signers.json`. Nxtlinq does not supply the project's signer
key, and Buzz must not automatically trust the repository-local `public.key`:
an actor able to replace project files could otherwise generate a new key and
self-authorize a replacement policy. The trust store must never contain the
signing private key.

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
3. Choose the **Agent workspace** with **Browse**. This is both the Gateway
   `--project` root and the ACP session working directory.
4. In **Nxtlinq authorization**, select **Recheck**. The check verifies the
   Gateway, trust material, signed project policy, and receipt destination.
5. When the check is ready, select **Enable**, save the Agent, and restart it if
   Buzz reports that a restart is required.

For an Agent that already has Nxtlinq enabled, changing its workspace or the
shared operator paths invalidates the prior readiness result. **Save changes**
stays disabled until **Recheck** succeeds. A successful recheck rebuilds the
stored wrapper arguments from that exact checked configuration, so the normal
update flow is:

```text
change workspace or shared operator paths
→ Recheck
→ Save changes
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
only when the structured wrapper command is the Nxtlinq Gateway. This prevents
an Agent configuration from granting itself the Gateway trust path.

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

- Buzz does not evaluate Nxtlinq manifests. Signature verification, policy
  evaluation, receipts, and the `--check` contract belong to the Gateway.
- Buzz owns safe installation/discovery, global public trust configuration,
  structured wrapper construction, launch-time trust derivation, and the host
  integrations that route supported operations through the Gateway.
- The Buzz Agent bridge protects its file tools and maps shell execution to a
  separate `terminal:execute` decision. A denied `read_file` therefore cannot
  be bypassed with `cat` through that shell tool.
- Conversational replies use a dedicated structured message tool so a denied
  terminal policy does not prevent the Agent from reporting the denial.
- This is not a complete OS sandbox. A downstream runtime or third-party MCP
  server with an independent filesystem, process, or network channel remains
  outside this Gateway path unless that channel is separately brokered.
- Full binary provenance/update lifecycle, remote trust-bundle distribution,
  Guardian policy composition, and a receipts timeline are intentionally
  deferred from this installation MVP.

See the Gateway's `docs/acp-host-integration.md` for its policy, receipt, and
conformance contract.
