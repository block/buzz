# 03 INGESTION PIPELINE

> Concrete local test cases for the corresponding feature. Add evidence to `testing/LOCAL_VALIDATION_LOG.md` after execution.

| ID | Test | Procedure | Expected result |
|---|---|---|---|
| TC-F03-001 | Incremental file change | Modify one test file. | Only affected chunks/symbols are reprocessed. |
| TC-F03-002 | Content-hash dedup | Re-ingest unchanged file. | No duplicate durable chunks are created. |
| TC-F03-003 | Deletion | Delete a source file. | Associated derived data is removed or invalidated. |

## Status
- [ ] Automated coverage implemented where appropriate
- [ ] Manual local validation completed
- [ ] Evidence recorded
