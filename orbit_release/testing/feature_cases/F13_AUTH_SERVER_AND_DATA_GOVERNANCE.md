# 13 AUTH SERVER AND DATA GOVERNANCE

> Concrete local test cases for the corresponding feature. Add evidence to `testing/LOCAL_VALIDATION_LOG.md` after execution.

| ID | Test | Procedure | Expected result |
|---|---|---|---|
| TC-F13-001 | Personal vs enterprise | Create personal and enterprise workspaces. | Data ownership boundaries are enforced. |
| TC-F13-002 | Policy gate | Mark a record local-only. | Record never enters outbound sync. |
| TC-F13-003 | Hosted processing gate | Deny hosted processing for a workspace. | Hosted model call is rejected/omitted while local processing continues. |
| TC-F13-004 | Device telemetry separation | Register a device. | Device metadata is stored in the identity/control plane and is absent from memory content. |
| TC-F13-005 | Delete/export | Run test deletion and export flows. | Policy-compliant data removal/export evidence is produced. |

## Status
- [ ] Automated coverage implemented where appropriate
- [ ] Manual local validation completed
- [ ] Evidence recorded
