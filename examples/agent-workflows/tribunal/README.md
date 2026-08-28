# Tribunal agent workflow example

This example demonstrates a generic durable multi-agent workflow with separate worker identities. It is configuration and contract material, not legal advice. The fixture is synthetic and contains no real personal data.

The coordinator executes workflow.yaml using the roster and budgets in manifest.yaml. Every worker returns an artifact validated by the referenced JSON Schema. The relay-visible run remains authoritative even if an internal agent runtime uses subagents or teams.
## Implementation status

workflow.yaml is the versioned target contract for the durable actions introduced by this example. Until run_agent, barrier, verify_artifact, ingest_document, and publish_artifact are added to buzz-workflow, validate it as YAML specification only; do not load it into the current WorkflowDef parser.
