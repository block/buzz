# 10 PACKAGING HARDENING

> Concrete local test cases for the corresponding feature. Add evidence to `testing/LOCAL_VALIDATION_LOG.md` after execution.

| ID | Test | Procedure | Expected result |
|---|---|---|---|
| TC-F10-001 | Clean install | Install on a clean Windows/macOS machine. | App launches without external DB/service dependencies. |
| TC-F10-002 | Restart recovery | Populate brain, restart, inspect state. | State survives restart. |
| TC-F10-003 | Offline recall | Disable network and recall known memory. | Local recall continues. |
| TC-F10-004 | Benchmark capture | Run local benchmark fixture. | Latency, memory and disk measurements are recorded. |

## Status
- [ ] Automated coverage implemented where appropriate
- [ ] Manual local validation completed
- [ ] Evidence recorded
