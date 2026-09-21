# Standalone forum agent invitation

The standalone forum composer reuses the remote mention preparation/publication
contract (`docs/remote-mention-routing.md`). An eligible owned relay nonmember
opens an explicit Invite/Cancel dialog. There is no implicit reference-only send.

Invite revalidates the captured exact recipients in the preparation phase,
performs the existing authorized bot-member add, awaits membership invalidation,
then performs a fresh publication-phase check against the captured destination.
A successful add alone is never authorization to publish.

Cancel resolves the pending submission without clearing its text or selected
identity. Retry can use the same exact key. Rejected adds and preparation errors
remain visible inside the dialog; final authorization errors remain visible in
the composer and retain its draft. Channel change/unmount cancels pending work,
and completions must still belong to the mounted captured channel. An add already
accepted by the relay is not rolled back on cancellation; no message is sent.

This adapter does not create/manage local agents or change notes/channel-less
surfaces. Regression coverage in `forum-agent-invitation.spec.ts` covers exact
p-tags, membership refresh before final authorization, cancel/retry, three add
failures, policy revocation before Invite and during add, and navigation during
an outstanding add. Browser fixtures are mock IPC; signed native ownership and
membership validation live in PR6, not in the frontend bridge.

## Dispatched reply recovery

After authorized dispatch, transport rejection recovers the captured text,
completed/uploaded media and exact selected mention refs under the source draft
key. This extends the standalone-forum follow-up noted in the shared routing
document: a cached A → B → A visit with no new intent no longer loses its reply.
The departed editor is never called. A pristine, empty current source visit may
adopt the recovered store entry; B cannot receive A's text or attachments.

The existing same-window draft authority fences recovery across visits. New text
(including edit → delete), completed-media changes/upload intent, selected refs,
explicit draft deletion, a new send or a community/identity reset revoke older
recovery. A different existing stored payload is not overwritten. Optimistic
clear and recovery are programmatic, not authored deletion; synchronous media
snapshots protect cleanup immediately after recovery. No recovery path repeats
publication, undoes an accepted add or releases another visit's pending send.

`ForumComposer.lifecycle.test.mjs` exercises ordinary and invited sends,
source-key re-entry, newer intent/deletion/media/refs, scope reset and late
completion. The browser spec additionally rejects deferred `send_channel_message`
after cached A → B → A with uploaded media and verifies recovery versus newer
text/deletion, B isolation and exact recipients on an explicit retry.

Recovery is not a durable in-flight send journal: renderer reload/crash while the
transport is pending can still lose the in-memory snapshot. Local in-flight
upload custody, cross-window coordination, native authorization and atomic
relay publication are outside this correction.
