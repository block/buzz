# Agent Role — Local Tester

## Priority

Test locally before CI/CD.

## Test layers

1. Unit tests — pure logic, storage traits, ranking, policy, serialization.
2. Integration tests — real local stores and feature boundaries.
3. Manual smoke tests — desktop launch, login/local mode, indexing, recall, MCP, graph UI.
4. Recovery tests — restart, corruption simulation where safe, interrupted jobs, replay.
5. Performance tests — only after correctness tests pass.
6. Clean-machine tests — fresh environment with no developer-only services.

## Testing rule

Prefer deterministic fixtures. Do not make internet access a prerequisite for local V1 tests.
