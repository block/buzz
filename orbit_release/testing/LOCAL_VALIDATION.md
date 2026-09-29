# ORBIT Local Validation Plan

## Gate A — Fresh machine

- Build from a clean checkout.
- Launch without PostgreSQL, Redis, Neo4j, FalkorDB, Docker, or another external DB service.
- Confirm `orbit_brain/` directories are created correctly.

## Gate B — Local memory

- Ingest a small repository.
- Change a source file.
- Confirm unchanged content is not re-embedded.
- Confirm deleted content is removed/forgotten across indexes.
- Restart and confirm the brain reopens.

## Gate C — Retrieval

- Test exact symbol lookup.
- Test semantic recall.
- Test current-vs-superseded architecture decisions.
- Test graph-assisted recall.
- Test token-budget packing and provenance.

## Gate D — Agents

- Connect one supported agent through MCP.
- Run a context retrieval call.
- Verify untrusted context fencing.

## Gate E — Identity

- Continue local without an account.
- Sign up through the hosted auth website.
- Return through the desktop deep link.
- Confirm secure local session restoration.
- Confirm device registration does not become memory content.

## Gate F — Sync

- Enable cloud sync for a test account.
- Make a local memory change.
- Sync to a second test device/profile.
- Confirm the logical memory is reconstructed locally.
- Disconnect network and confirm local recall continues to work.

## Gate G — Enterprise

- Create separate personal and enterprise workspaces.
- Confirm ACL filtering before retrieval/graph expansion.
- Test local-only data classification.
- Test sync-denied data.
- Test hosted-processing-denied data.
- Test retention/delete/export hooks.
