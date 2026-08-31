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
