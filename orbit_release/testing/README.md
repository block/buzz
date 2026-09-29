# ORBIT Testing Package

Testing is intentionally staged so local correctness is established before CI/CD.

## Files

- `TEST_STRATEGY.md` — overall strategy.
- `TEST_MATRIX.md` — feature-level test coverage matrix.
- `LOCAL_VALIDATION.md` — manual local verification order.
- `LOCAL_VALIDATION_LOG.md` — evidence log; append results as features complete.
- `feature_cases/` — concrete test cases for F01–F13.

## Rule

A feature cannot be marked complete until its automated and required manual tests have passed locally.
