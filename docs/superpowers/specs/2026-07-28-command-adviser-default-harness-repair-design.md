# Command Adviser Default Harness Repair

## Goal

Make the saved application default harness authoritative for existing managed
agents that do not specify a persona or instance runtime, and ensure a harness
change takes effect without leaving the old process answering configuration
nudges.

## Design

Harness resolution uses one precedence order everywhere:

1. Explicit per-agent command override.
2. Materialized agent runtime.
3. Linked persona runtime.
4. Saved global `preferred_runtime`.
5. Built-in `buzz-agent` fallback.

The resolver remains pure by accepting the preferred runtime as an argument.
Spawn, status, and summary paths pass the already-loaded global preference.
Callers that deliberately have no application context pass `None`.

When Edit Agent changes the harness of an active local agent, the frontend
restarts that agent after the update succeeds. A stopped agent remains stopped
and retains the existing “Start now” affordance. A restart failure is reported
without rolling back the saved configuration.

Command Adviser managed agents use one ACP worker by default. Existing Command
Team records with the legacy value `24` are migrated to `1`; deliberate values
other than `24` are preserved.

## Verification

- Rust unit tests prove the full runtime precedence, including the global
  preferred runtime and final Buzz fallback.
- Frontend tests prove an active harness edit requests exactly one restart and
  a stopped-agent edit does not start it implicitly.
- Migration tests prove only the legacy Command Team parallelism is reduced.
- Focused Rust and desktop tests run before broader relevant checks.

