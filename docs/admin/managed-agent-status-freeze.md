# Managed-agent status freeze response

## Incident signature

On Linux, the desktop may be reported as unresponsive while managed-agent
status refreshes continue to accumulate. Confirm the signature before changing
state:

- `buzz-desktop` has an unusually high or steadily increasing thread count.
- Several identical CLI login probes are live at once.
- The UI requests managed-agent runtime status faster than prior requests
  finish.

The August 2026 incident reached 544 desktop threads, including 521 Tokio
workers. A five-second UI polling cadence repeatedly entered a runtime listing
path that performed multiple roughly two-second CLI readiness probes while
runtime-management locks were held.

## Safeguards

Runtime status polling is completion-based and single-flight in both the UI and
backend. The backend snapshots runtime generations under its locks, releases
them before any readiness process starts, then revalidates exited generations
before persisting their delayed state. Login probes are cached by effective command and relevant
environment, invalidated on successful configuration writes, limited to one
active probe per effective key while different keys remain independent, and
killed and reaped after five seconds. Output is drained continuously while
at most 64 KiB is retained.

Unexpected local listener exits use three bounded recovery attempts after 5
seconds, 30 seconds, and 2 minutes. Recovery is suppressed while an agent has
active work. The desktop reports both confirmed recovery and retry exhaustion;
a successful restart command alone is not considered recovery.

## Release procedure

1. Record the current executable checksum, process thread count, configured
   listener count, and listener health.
2. Build from the Hermit environment through Tauri's production pipeline. For
   an official release, first provide `BUZZ_UPDATER_PUBLIC_KEY` and
   `BUZZ_UPDATER_ENDPOINT`, run
   `cd desktop && node scripts/build-release-config.mjs`, then run
   `pnpm tauri build --verbose --ci --bundles deb,appimage --features mesh-llm --config src-tauri/tauri.release.conf.json`.
   For a local incident rollout that must not claim updater signing metadata,
   use the base configuration with
   `cd desktop && pnpm tauri build --verbose --ci --bundles deb --features mesh-llm`.
   Both commands run the configured frontend build and compile the desktop with
   Tauri's production custom protocol. A plain `cargo build --release` is not a
   releasable artifact because it does not prove that the production frontend
   was embedded.
3. Run frontend tests and type checking, Rust formatting, the complete Rust
   suite, and repository checks. Stop if a new failure appears.
4. Copy the installed executable to a uniquely timestamped rollback path.
5. Copy the verified artifact beside the installed executable, verify its
   checksum, then rename it over the destination on the same filesystem.
6. Restart Buzz once. Do not modify agent stores, keys, relay configuration, or
   listener data during the cutover.
7. Confirm all expected listeners and relays, fewer than 80 desktop threads,
   and no upward thread trend under repeated status refreshes.

Rollback by stopping Buzz, atomically restoring the timestamped executable,
and restarting it. Retain the incident artifact and monitor log until the
replacement has passed the 24-hour checkpoint.

## Monitoring checkpoints

At 15 minutes, 2 hours, and 24 hours after deployment, record the executable
checksum, PID, thread count, file-descriptor count, listener count, unhealthy
listeners, and duplicated readiness processes. Roll back immediately if the UI
hang recurs, thread count reaches 80, thread or descriptor counts trend upward,
listeners are missing, or the same effective readiness command runs
concurrently.
