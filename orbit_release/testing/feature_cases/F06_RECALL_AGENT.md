# 06 RECALL AGENT

> Concrete local test cases for the corresponding feature. Add evidence to `testing/LOCAL_VALIDATION_LOG.md` after execution.

| ID | Test | Procedure | Expected result |
|---|---|---|---|
| TC-F06-001 | Parser fixture | Run each supported transcript parser on a fixture. | Events are normalized into the shared memory/event model. |
| TC-F06-002 | Incremental replay | Process same transcript twice. | No duplicate durable events are created. |

## Status
- [ ] Automated coverage implemented where appropriate
- [ ] Manual local validation completed
- [ ] Evidence recorded
