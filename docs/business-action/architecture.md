# Architecture

```text
V5 deterministic anomaly run
  -> Business Action Service (finding lifecycle + catalog mapping)
  -> Action Proposal (system suggestion)
  -> Business Dock preview + BusinessSession/CSRF/Origin confirmation
  -> Work Item (internal follow-up only)
  -> Approval Draft (draft-only material)
  -> stop

Business Agent -> AgentReadDelegation -> Business Read MCP -> six read-only action tools
```

Authority-changing APIs and MCP tools do not exist. Fail-closed guard routes record attempted approve, reject, execute, apply, commit, post, or ERP sync operations as blocked. PostgreSQL stores lifecycle state and relational constraint mirrors; stable hashes bind findings, proposals, previews, and drafts to their source versions.

Condition status belongs to the deterministic engine. Review status belongs to users. Keeping them separate prevents an acknowledgement or resolution from being mistaken for a business correction. Buzz carries only summaries, identifiers, status, `biz://` links, and trace IDs; the detailed workflow remains in the independent Business System surface.
