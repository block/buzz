# 02 EMBEDDING ENGINE

> Concrete local test cases for the corresponding feature. Add evidence to `testing/LOCAL_VALIDATION_LOG.md` after execution.

| ID | Test | Procedure | Expected result |
|---|---|---|---|
| TC-F02-001 | Local embedding | Run embedding in local mode with network disabled. | Embedding succeeds and emits no network request. |
| TC-F02-002 | Reranker | Run reranking against deterministic candidates. | Ordering is deterministic within expected tolerance. |
| TC-F02-003 | Provider abstraction | Swap provider implementation behind trait. | Higher-level retrieval code does not depend on vendor-specific types. |

## Status
- [ ] Automated coverage implemented where appropriate
- [ ] Manual local validation completed
- [ ] Evidence recorded
