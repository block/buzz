# Send later deployment

[NIP-SL](nips/NIP-SL.md) defines send later: what a client publishes and what
a relay does with it. This guide holds the rules for running it: several relay
processes on one storage, changing the executor key, the order in which a
deployment turns it on, and how to switch it off or roll back. MUST and SHOULD
bind a deployment as they bind a relay in the NIP.

## Several relay processes

A deployment MAY run any number of relay processes against the same storage,
and any of them MAY release any send.

- Every change to the state of a send (accept, replace, cancel, release, an
  early end by the relay, `missed`) MUST be a conditional change in the shared
  durable storage. It MUST apply only if the send is in the state and at the
  generation that the process read. The process whose change applies owns the
  outcome. Every other process finds nothing to do.
- A process MUST NOT rely on state held only in its own memory, such as a
  NIP-FI deny set, to decide whether a send may be accepted or released.
- The message MUST be stored at most once, however many processes attempt the
  release.
- A send and the message it becomes MUST be committed in the same storage
  transaction. A deployment that partitions its storage MUST keep both in the
  same partition.

## Changing the executor key

An executor key changes in three steps. Every process MUST complete a step
before any process begins the next.

1. Every process holds the new key pair and lists the new key as `standby`.
2. Every process marks the new key `current` and the old key `standby`.
3. Every process marks the old key `retiring`.

A process accepts an instruction under its `current` key and under every key
it lists as `standby` (check 10 of the NIP's
[acceptance checks](nips/NIP-SL.md#acceptance-checks)). So while the
processes disagree about a step, each of them accepts an instruction under the
key that any of them advertises as `current`, and each lists the key that any
of them signs a status with.

Every process holds the secret key of each key it lists that still has a
pending instruction. A `retiring` key stays listed for as long as the NIP's
retention rules require.

## Turning it on

A deployment MUST introduce send later in this order:

1. Every relay process that shares the storage, and every tool that reads or
   deletes stored events, enforces the NIP's read rules for both kinds and
   refuses both kinds at ingest. No process advertises send later. A
   deployment MUST NOT roll back below this version once step 2 has begun.
2. Send later is enabled for a community. Only now may a process accept
   `kind:30360`, publish `kind:30361` or serve the descriptor.

In a community that requires NIP-FI, an operator MUST NOT enable send later
unless every issuer the community authorizes sends a disconnect command when
an identity loses access. An issuer that only stops issuing assertions does
not stop a pending send. Without NIP-FI, bans, timeouts and the release checks
are what stop one.

## Switching off

A relay SHOULD provide two controls for each community:

- **Accepting.** When it is off, the descriptor carries `"accepting": false`
  and the relay refuses every instruction that holds a message, whether it
  starts a send or replaces one. Pending sends are still released, cancels are
  still accepted, and reads still work.
- **Releasing.** When it is off, nothing is released. A send whose release
  window passes meanwhile ends as `missed`.

While a pending send, a readable instruction or a status exists in a
community, a relay with send later MUST keep serving reads and accepting
cancels there, and MUST end every pending send with an outcome.

## Rolling back

A deployment SHOULD switch off the two controls rather than roll back to the
step 1 version while sends exist.

The step 1 version keeps both kinds private and releases nothing, so no
message is posted after a cancel. It also refuses every instruction, including
a cancel, and publishes no status, so the guarantee in "Switching off" is
suspended for as long as it runs. When a version with send later runs again,
it MUST end each send whose release window has passed as `missed`.

A version without send later also cancels nothing:

- A ban or a channel deletion that it records, and that still stands at
  release, ends the send then, because the relay applies the `banned` and
  `channel_deleted` rows of the NIP's [Status](nips/NIP-SL.md#status) table
  again at release.
- A NIP-FI disconnect command leaves nothing to apply again. A version that
  does not apply "Deferred actions" in [NIP-FI](nips/NIP-FI.md) answers the
  command `200`, keeps no disconnect record and cancels nothing. The sends of
  the target and of its agents would be released once a version with send
  later runs again.

A deployment therefore MUST NOT run a version that does not apply NIP-FI
"Deferred actions" while a send whose recorded admission names an issuer is
pending. Either its step 1 version applies that section to the stored sends,
or the deployment does the following, in this order:

1. It switches Accepting off for every community on every process, and keeps
   it off on every process that starts before step 4.
2. It waits until every instruction that a process began to check before
   step 1 took effect there has been accepted or refused. Such an instruction
   can have passed check 8, and would otherwise commit after step 3.
3. It ends every such send as `cancelled` with reason `withdrawn`, and a
   process with send later stores those statuses.
4. It confirms that no such send is pending. Only then does it start a
   process without send later.

A relay that evaluates check 8 inside the atomic change of acceptance,
serialized against the change that switches Accepting off, has nothing to
wait for in step 2 once step 1 has committed.
