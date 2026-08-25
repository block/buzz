# Authorization

MCP scope is necessary but not sufficient. Production requests must pass three
gates:

1. Business IAM: active Human/Agent principal, role/direct grant, action and data-scope decision.
2. AgentReadDelegation: correct Turn, IAM decision, scope, binding, user and call budget.
3. Business Read authorization: service identity and delegation context.
4. Business System authorization: the current Enterprise user's real role and
   data scopes.

For each legal entity, brand, warehouse, customer, supplier, salesperson and
period dimension, the Business System computes:

```text
effective scope = requested scope intersection authorized scope
```

It must not trust channel membership, Agent identity, `biz://`, prompt text, or
input role fields. Exact-id misses and authorization denials have the same
external result. Search returns only accessible rows.

No production fallback grants all data. `agent_read_delegations` is a temporary
credential ledger, not the authority source. If Business IAM has no matching
active principal and effective grant, issuance fails closed.
