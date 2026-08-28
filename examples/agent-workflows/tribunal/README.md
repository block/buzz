# Tribunal agent workflow example

This example demonstrates a generic durable multi-agent workflow with separate worker identities. It is configuration and contract material, not legal advice. The fixture is synthetic and contains no real personal data.

The coordinator executes workflow.yaml using the roster and budgets in manifest.yaml. Every worker returns an artifact validated by the referenced JSON Schema. The relay-visible run remains authoritative even if an internal agent runtime uses subagents or teams.
## Implementation status

workflow.yaml is a valid WorkflowDef and its dependency graph is validated at load time. It requires the durable scheduler: the legacy sequential executor rejects it before mutating run state. Do not use it for execution until task dispatch, barriers, checkpoints, artifact validation, and persistent approval are connected.
