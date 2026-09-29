# 05 KNOWLEDGE GRAPH

> Concrete local test cases for the corresponding feature. Add evidence to `testing/LOCAL_VALIDATION_LOG.md` after execution.

| ID | Test | Procedure | Expected result |
|---|---|---|---|
| TC-F05-001 | Entity relation creation | Ingest deterministic fixtures. | Expected nodes/edges exist with provenance. |
| TC-F05-002 | Temporal invalidation | Introduce a replacement fact. | Old relation becomes invalid/superseded without losing history. |
| TC-F05-003 | Graph-assisted recall | Ask a multi-hop dependency question. | Relevant neighborhood contributes evidence. |

## Status
- [ ] Automated coverage implemented where appropriate
- [ ] Manual local validation completed
- [ ] Evidence recorded
