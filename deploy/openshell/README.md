# Codex authentication for OpenShell agents

Do not copy a host `~/.codex/auth.json` into Buzz agent sandboxes. A copied
ChatGPT login shares revocable refresh material with the host, exposes live
tokens in every workspace, and stops every copy when that login is revoked.

For several unattended agents, use one OpenShell provider backed by a dedicated
Codex login. OpenShell keeps the refresh token as gateway-only secret material,
rotates the short-lived access token, and injects only opaque, endpoint-bound
placeholders into attached sandboxes. Codex still gets its expected
`auth.json` shape, but that file contains no usable OpenAI credential.

This requires OpenShell provider refresh support (available in OpenShell
0.0.116). If provider refresh is unavailable, log in independently inside each
persistent sandbox with `codex login --device-auth`; OpenAI documents copying
`auth.json` only as a fallback for headless environments.

## Configure the provider

Create a dedicated temporary Codex home so the gateway does not share the
host CLI's refresh-token lineage:

```bash
auth_home="$(mktemp -d)"
CODEX_HOME="$auth_home" codex login --device-auth
deploy/openshell/configure-codex-provider.sh \
  --auth-file "$auth_home/auth.json" \
  --gateway my-gateway
```

After the script reports a successful rotation, remove the temporary directory
or keep it in encrypted operator storage for disaster recovery. Never commit
it. A future reauthorization uses a fresh dedicated login followed by
`--reset-refresh`.

The script imports `providers/codex-buzz.yaml`, stores the bootstrap access token
and account ID in the gateway, stores the refresh token as non-injectable secret
refresh material, and forces an initial rotation. It does not put token values
on the command line or upload the source auth file.

## Create or migrate sandboxes

Prefer attaching the provider when the sandbox is created. The base policy must
not also contain the image's generic L4-only `codex` rule; the provider supplies
the inspected Codex endpoints and binaries:

```bash
openshell sandbox create \
  --name kestrel \
  --from your-buzz-agent-image \
  --policy your-buzz-agent-policy.yaml \
  --provider codex-buzz \
  --no-auto-providers \
  --detach
```

OpenShell 0.0.116 rejects provider attachment when an existing sandbox was
created with an L4-only Codex rule, even after that rule is replaced in a live
policy revision. Recreate such a sandbox from its source configuration with the
provider attached at creation time; preserve its Buzz identity and any intended
workspace artifacts through the normal backup/restore path. Do not bypass this
guard with `allow_uninspected_credentials`.

For a sandbox whose original policy already has no overlapping Codex rule, an
attachment followed by a restart is sufficient:

```bash
openshell sandbox provider attach kestrel codex-buzz
openshell sandbox stop kestrel
openshell sandbox start kestrel
```

Include `materialize-codex-auth.mjs` in the sandbox image and run it immediately
before starting `codex-acp`:

```bash
export CODEX_HOME=/sandbox/runtime/.codex
node /opt/buzz/materialize-codex-auth.mjs
exec buzz-acp
```

The materializer refuses raw credentials and writes `auth.json` atomically with
mode `0600`. Its access token and account ID are OpenShell placeholders; its
refresh token is a sentinel because refresh belongs exclusively to the gateway.
The provider policy permits those placeholders only for Codex/Node processes
and the declared OpenAI endpoints.

Verify without displaying credentials:

```bash
openshell provider refresh status codex-buzz \
  --credential-key CODEX_AUTH_ACCESS_TOKEN
openshell sandbox exec -n kestrel -- codex login status
openshell sandbox exec -n kestrel -- codex exec --skip-git-repo-check \
  'Reply with AUTH_OK only.'
```

## Reauthorize

If refresh status says `reauthorization_required`, make a new temporary
dedicated login and replace the gateway-owned refresh state explicitly:

```bash
auth_home="$(mktemp -d)"
CODEX_HOME="$auth_home" codex login --device-auth
deploy/openshell/configure-codex-provider.sh \
  --auth-file "$auth_home/auth.json" \
  --gateway my-gateway \
  --reset-refresh
```

Opaque placeholders remain stable, so attached sandboxes do not need a new
`auth.json`. Restart the long-running agent process only if it has retained a
failed Codex child.

## References

- [OpenAI Codex authentication](https://developers.openai.com/codex/auth)
- [OpenShell provider profiles and gateway-managed refresh](https://github.com/NVIDIA/OpenShell/blob/main/docs/providers/profiles.mdx)
- [OpenShell's Codex Gator provider example](https://github.com/NVIDIA/OpenShell/tree/main/scripts/agents/gator)
