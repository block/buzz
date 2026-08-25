# Fixed Action Adapter Contract

The V6.5 `BusinessActionAdapter` exposes only:

```text
capabilities()
preflight()
verify_current_state()
verify_postcondition()
describe_compensation()
```

There is no `execute()` or `compensate()` method. An adapter is compiled and
registered for one explicit, versioned business action; catalog/configuration
cannot supply an arbitrary URL, HTTP method or JSON body.

The associated network policy permits exact HTTPS origins, GET, and only
side-effect-free POST paths `/read`, `/search`, `/preflight` and `/authorize`.
PUT, PATCH, DELETE and business-write POST paths are not representable.
