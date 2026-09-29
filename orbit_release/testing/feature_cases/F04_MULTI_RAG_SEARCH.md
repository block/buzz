# 04 MULTI RAG SEARCH

> Concrete local test cases for the corresponding feature. Add evidence to `testing/LOCAL_VALIDATION_LOG.md` after execution.

| ID | Test | Procedure | Expected result |
|---|---|---|---|
| TC-F04-001 | Exact lookup | Search for a known symbol/error/path. | Lexical/code retrieval contributes the relevant evidence. |
| TC-F04-002 | Semantic recall | Ask for a paraphrase of a stored memory. | Vector retrieval contributes relevant evidence. |
| TC-F04-003 | Temporal conflict | Create current and superseded memories. | Current valid memory outranks superseded memory for current-state queries. |
| TC-F04-004 | Context pack | Run retrieval with small token budget. | Output is deduplicated, provenance-bearing and within budget. |

## Status
- [ ] Automated coverage implemented where appropriate
- [ ] Manual local validation completed
- [ ] Evidence recorded
