# Require NIP-FI on an existing community

`buzz-admin nip-fi-upgrade` makes one signed HTTP call to the existing relay
operator API. It needs no database access, application startup, sponsorship or
billing workflow. `enable` accepts the saved upgrade tuple; `status` reads its
durable receipt. Both require an authorized deployment operator key.
The [upgrade API contract](community-auth-upgrades.md) defines the receipt and
policy semantics used below.

This command is a trusted operator surface. The operator must independently
authenticate the community owner and verify the owner's registered public key
before putting it in `actor_pubkey`. A caller-supplied public key is not proof
of the owner's identity. The relay still checks current ownership, community
identity, issuer trust and the upgrade admission gate. This command does not
replace an application's owner authentication or admission policy.

## Before the test

Use a disposable staging community whose policy is not yet required. Verify its
UUID, canonical lowercase host, current owner and active issuer binding against
authoritative records. Use the current owner's key for both public-key fields.
The issuer must match the relay's provisioning configuration and active trust.

Verify `BUZZ_NIP_FI_MODE=shadow` on **every serving replica** before connecting
the test client. Shadow observes optional communities and enforces their stored
required policy after an upgrade. `off` ignores stored requirements; `enforce`
already requires sign-in everywhere. Either mode invalidates this transition
test. Keep shadow mode unchanged throughout the test and record it in evidence.

All serving relay replicas must support persisted required policy and closing
existing sessions after an upgrade. Apply their required migrations and finish
the rollout before the test. The deployment must already have authorized
`BUZZ_NIP_FI_UPGRADES_ENABLED=true`; this command does not change that gate.
Only the deployment's authorized operator may enable that gate, after verifying
the fleet prerequisites in the API contract. Record that operator's approval.
Use a client that supports required-policy discovery and federated sign-in.

Identify the configured `RELAY_OPERATOR_API_ORIGIN` and a key in
`RELAY_OPERATOR_PUBKEYS`. The origin is the operator API origin, not the target
community URL. Use HTTPS outside a trusted internal mesh. Provision the key as
a protected file/Secret through the deployment's existing secret process; never
paste it into arguments, logs or a committed manifest. Do not borrow a client
or managed agent key.

## Save the operation, then leave the client connected

Save `request.json` with one newly generated operation UUID and the verified
values. Keep this file unchanged for retries. The command refuses noncanonical
hosts or public keys before sending a POST. These are example values only:

```json
{
  "operation_id": "deecfc7a-9ec2-4ec5-b15b-9e567ad59740",
  "community_id": "00fb4822-7930-40fc-9eed-315926f83ca7",
  "expected_host": "test.communities.example.org",
  "actor_pubkey": "<verified owner public key: 64 lowercase hex characters>",
  "expected_owner_pubkey": "<same verified owner public key>",
  "expected_issuer": "https://issuer.example.org"
}
```

Open the test community in an existing client and confirm it can read and send
a message without federated sign-in. Keep that session connected. Record the
client version, relay image digests and time, plus the community's `/info`
response showing `requires_nip_fi: false`. Do not sign out, restart the client,
or change relay configuration during the acceptance test.

## Run one pod

Use a relay image containing this command, pinned to its verified digest. The
normal relay image includes `/usr/local/bin/buzz-admin`. Check the command's
`--help` before the test if using a previously built image.

Select the staging context and namespace explicitly. Create the input ConfigMap
from the saved request, and use an existing Secret with a `key` entry:

```sh
kubectl --context "$STAGING_CONTEXT" -n "$NAMESPACE" create configmap nip-fi-upgrade-input \
  --from-file=request.json
```

Replace the image, origin and Secret name below with reviewed deployment
values. Save as `upgrade-pod.yaml`. This is a single Pod, with no Job controller,
retry policy or application/service labels. It mounts only the input and key.
Use the deployment's approved network/mesh settings if its operator API needs
them; confirm the pod can reach that API without adding it to serving workloads.
If a mesh sidecar is injected, use the platform's approved one-shot completion
mechanism (for example, native sidecars). An ordinary sidecar can keep this Pod
running after the command exits; retain the upgrade container's exit status and
logs, and arrange sidecar shutdown without weakening required network policy.

```yaml
apiVersion: v1
kind: Pod
metadata:
  name: nip-fi-upgrade-enable
spec:
  restartPolicy: Never
  activeDeadlineSeconds: 60
  automountServiceAccountToken: false
  securityContext:
    runAsNonRoot: true
    runAsUser: 1000
    runAsGroup: 1000
    fsGroup: 1000
  containers:
    - name: upgrade
      image: ghcr.io/block/buzz@sha256:<verified-digest>
      command: [/usr/local/bin/buzz-admin]
      args:
        - nip-fi-upgrade
        - --origin
        - https://operator.example.org
        - --key-file
        - /operator/key
        - enable
        - --request
        - /input/request.json
      volumeMounts:
        - {name: operator, mountPath: /operator, readOnly: true}
        - {name: input, mountPath: /input, readOnly: true}
  volumes:
    - name: operator
      secret:
        secretName: reviewed-upgrade-operator
        defaultMode: 0440
    - name: input
      configMap:
        name: nip-fi-upgrade-input
```

```sh
kubectl --context "$STAGING_CONTEXT" -n "$NAMESPACE" create -f upgrade-pod.yaml
kubectl --context "$STAGING_CONTEXT" -n "$NAMESPACE" logs -f pod/nip-fi-upgrade-enable -c upgrade
kubectl --context "$STAGING_CONTEXT" -n "$NAMESPACE" get pod nip-fi-upgrade-enable -o json
```

The last command records the pod UID, observed image ID and container exit code.
Do not use Pod phase alone: the expected accepted outcome in this relay version
is exit **10**, which Kubernetes labels Failed. This is acceptance, not a command
failure or a certified fleet-wide connection drain.

| Exit | Meaning |
| --- | --- |
| 10 | Expected success: valid matching receipt, `requires_nip_fi: true`, state `pending` |
| 12 | Valid acceptance receipt, target/trust currently `unavailable` |
| 5 | Request error or unconfirmed result; acceptance may already have happened |

This relay version never reports `active`. The command reserves exit 0 for a
future valid `active` receipt with no pending reasons; do not wait for it here.
`connection_closure_unverified` is expected even after successful socket sweeps:
the receipt does not certify fleet-wide closure. Continue with the client test.
Retain and investigate other/unknown pending reasons; they remain unverified.

Each HTTP call has a 30-second timeout, no automatic retry and no redirects.
The command verifies the returned operation, operator, required policy and
positive generation. For enable it also verifies every saved tuple field.

## Confirm policy and recover the same operation

Create another pod from the same manifest with a new pod name and replace the
`enable --request /input/request.json` arguments with:

```yaml
        - status
        - --operation-id
        - deecfc7a-9ec2-4ec5-b15b-9e567ad59740
```

Use the actual saved operation UUID, same API origin and same operator key.
Compare the receipt's complete tuple and generation with the saved evidence.
A successful receipt must report `requires_nip_fi: true`. Also read the target
community's `/info` and confirm it advertises required policy.

After timeout, response loss or pod termination, status may recover an accepted
receipt. If it cannot confirm acceptance, repeat enable with the **same saved
request** and a new pod name. Never create a new operation UUID to escape an
uncertain result. `pending` and `unavailable` do not undo required policy; retain
their state and reasons. A pending receipt with `connection_closure_unverified`
does not prevent the manual client test from passing on observed evidence.
An unavailable target or unknown obligations require investigation.
Turning off admission, deleting the pod or rolling back the command cannot undo
an accepted policy change.

## Observe the existing client

Record `BUZZ_COMMUNITY_REVALIDATE_INTERVAL_SECS` on every serving replica (default
30 seconds, clamped to 1–300). Before the test, choose and record a finite client
reconnect/sign-in budget from that client's configured reconnect behavior. Wait
at least the largest replica interval plus that client budget after acceptance.
Record the resulting observation deadline. If sign-in has not appeared by then,
record the test as failed or blocked with logs; do not wait indefinitely.

1. After the upgrade, verify the already-connected client demands federated
   sign-in before continuing to read or send in the community. Let the client
   handle the disconnect/reconnect. Do not manually sign out or restart it.
2. Complete sign-in with the test user's allowed issuer, then verify reading and
   sending work again in that same community.
3. Record a failure if access continues without sign-in, sign-in never appears,
   or access does not resume. A changed `/info` response alone is not a pass.

Record the exact invocation/manifest, saved request, operator public key,
community UUID/host, pod UID/observed image ID, all serving relay versions and
shadow modes, revalidator intervals, client reconnect budget/deadline,
client version, pre/post `/info`, receipt/generation/state, timestamps and observed
client behavior. Keep secret key bytes and authorization headers out of evidence.
Mark unexecuted steps as **not run**, and keep policy confirmation separate from
the client result. Export logs and pod metadata before deleting the temporary
pods and input ConfigMap. Use a fresh staging community for another first-upgrade
test; do not clear persisted policy to reset this one.
For a mistaken acceptance, stop the caller and follow the API contract's
containment procedure. Archive or isolate the affected community through the
deployment's authorized operator process; there is no policy rollback command.
