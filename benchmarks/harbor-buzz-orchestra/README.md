# Harbor Buzz Orchestra

Out-of-tree Harbor agent for benchmark trials in which one orchestrator coordinates a manifest-defined team through Buzz.

The initial package establishes the custom-agent import, immutable manifest, provisioning, and runtime contracts. Model names and staffing are manifest data; the adapter contains no fixed roster.

```bash
uv run harbor trials start \
  -p /path/to/task \
  --agent harbor_buzz_orchestra:BuzzOrchestraAgent \
  --agent-kwarg manifest=/path/to/condition.yaml
```

The provisioner/runtime integration must be configured before an actual trial; until M1 wiring lands, `run()` fails explicitly rather than bypassing Buzz.
