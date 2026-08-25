# Architecture

```text
trusted Buzz event
  -> active user/binding + one-turn read Delegation
  -> business-read-mcp (16 fixed tools)
  -> independent Business Read API final authorization
  -> deterministic analytics + versioned rules
  -> bounded Finding/Evidence/ResourceRef result
  -> Agent fact/rule/suggestion explanation
  -> ACP host-signed Buzz reply + response audit
```

The MCP cannot accept SQL, arbitrary URLs, joins, formulas or generic query
text. The Business API owns final data authorization. Analytics joins only
stable ids and comparable currency/unit values. The ACP host, not the model,
publishes the final answer, so the anomaly Agent receives no write tool or
signing credential.

The current evaluator is stateless and on-demand. Persisted runs/findings,
scheduling, cancellation/progress APIs and shared result caching are not
implemented.
