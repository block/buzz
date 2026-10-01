# 07 MCP SERVER TOOLS

> Concrete local test cases for the corresponding feature. Add evidence to `testing/LOCAL_VALIDATION_LOG.md` after execution.

| ID | Test | Procedure | Expected result |
|---|---|---|---|
| TC-F07-001 | Tool contract | Call each orbit.* tool with valid input. | Schema and response contract are stable. |
| TC-F07-002 | Context fencing | Return untrusted retrieved text containing an instruction. | It is treated as data, not as executable user instructions. |
| TC-F07-003 | Delete memory | Invoke delete tool on a fixture memory. | Delete propagates through the memory/storage policy. |

## Status
- [x] Automated coverage implemented where appropriate
- [x] Manual local validation completed
- [x] Evidence recorded
