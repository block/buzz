# Future CI/CD Boundary

CI/CD is intentionally not the first validation environment.

## Add CI/CD only after

- F01–F10 local V1 features pass their local gates.
- Clean-machine packaging is validated.
- Local benchmarks are repeatable.
- Test commands are stable and documented.

## Future pipeline stages

1. formatting/static checks
2. unit tests
3. integration tests
4. deterministic fixture tests
5. desktop build
6. packaging validation
7. benchmark smoke test
8. artifact signing/release automation
9. hosted-service tests after F11–F13 are introduced

The future CI system should run the same commands that have already passed locally. Do not create CI-only implementations that bypass local behavior.
