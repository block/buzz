# 06 RECALL AGENT

> Concrete local test cases for the corresponding feature. Add evidence to `testing/LOCAL_VALIDATION_LOG.md` after execution.

| ID | Test | Procedure | Expected result |
|---|---|---|---|
| TC-F06-001 | Parser fixture | Run each supported transcript parser on a fixture. | Events are normalized into the shared memory/event model. |
| TC-F06-002 | Incremental replay | Process same transcript twice. | No duplicate durable events are created. |

## Automated Test Mapping

| Test Case | Test Function | Result |
|---|---|---|
| TC-F06-001 | `tests/f06_recall_agent_tests.rs::tc_f06_001_all_9_parsers_fixtures` | ✅ PASS |
| TC-F06-002 | `tests/f06_recall_agent_tests.rs::tc_f06_002_incremental_replay_and_idempotency` | ✅ PASS |
| (extra) | `tests/f06_recall_agent_tests.rs::test_secret_redaction_during_transcript_recall` | ✅ PASS |
| (extra) | `tests/f06_recall_agent_tests.rs::test_policy_eligible_context_filter` | ✅ PASS |

## Status
- [x] Automated coverage implemented where appropriate
- [x] Manual local validation completed
- [x] Evidence recorded
