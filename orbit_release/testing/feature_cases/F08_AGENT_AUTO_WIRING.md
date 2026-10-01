# 08 AGENT AUTO WIRING

> Concrete local test cases for the corresponding feature. Add evidence to `testing/LOCAL_VALIDATION_LOG.md` after execution.

| ID | Test | Procedure | Expected result |
|---|---|---|---|
| TC-F08-001 | Connect harness | Connect one supported agent. | Config and skill files are written idempotently. |
| TC-F08-002 | Reconnect | Run connect twice. | No duplicate/broken configuration is produced. |

## Status
- [x] Automated coverage implemented where appropriate
- [x] Manual local validation completed
- [x] Evidence recorded
