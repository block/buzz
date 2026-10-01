# 01 STORAGE FOUNDATION

> Concrete local test cases for the corresponding feature. Add evidence to `testing/LOCAL_VALIDATION_LOG.md` after execution.

| ID | Test | Procedure | Expected result | Status |
|---|---|---|---|---|
| TC-F01-001 | Fresh local store creation | Launch on a clean machine with no DB server running. | SQLite, LanceDB and graph directories initialize; app remains usable offline. | PASS |
| TC-F01-002 | Cross-store identity | Create a chunk and related graph evidence. | All derived records reference the same authoritative IDs/provenance. | PASS |
| TC-F01-003 | Delete propagation | Delete/forget a memory. | Metadata, vector and graph references are removed or invalidated according to policy. | PASS |
| TC-F01-004 | Restart recovery | Write data, exit, relaunch. | Brain reopens with the same valid state and no external DB setup. | PASS |

## Status
- [x] Automated coverage implemented where appropriate (`crates/buzz-db/tests/f01_storage_foundation_tests.rs`)
- [x] Manual local validation completed
- [x] Evidence recorded (`testing/LOCAL_VALIDATION_LOG.md`)
