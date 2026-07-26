# Agent health view

The owner-facing health card is a projection of data Buzz already owns. It
does not add a second runtime capability table or infer health from names.

Current sources:

- managed-agent summary: avatar configuration, saved instructions presence,
  configuration update time, runtime, provider, model, response access, process
  start time, and configuration warnings;
- channel query: current visible channel memberships;
- presence query: current relay presence.

Known contract gaps are shown as `Unavailable`:

- managed-agent configuration has an update timestamp but no saved version;
- Buzz does not persist a per-agent timestamp for the last successful mention.

`Last run` is explicitly the latest managed process start. It is not presented
as the latest successful turn. A future turn-history contract should replace
that projection rather than silently changing its meaning.

This is a focused first slice of the broader readiness manifest proposed in
issue #2931. Runtime capabilities, authentication readiness, observer health,
effective permissions, and tool risk classification remain outside this view
until their authoritative contracts are wired through.
