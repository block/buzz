# Offline verifier preparation

Terminal-Bench 2.1 verifiers commonly install Python dependencies at grading time. That is not a valid measurement path in Block's network environment, so benchmark tasks use generated **prepared images**.

The preparation contract is deliberately narrow:

- `tests/` remains byte-identical to the frozen source task.
- Python wheels are resolved host-side through Block Artifactory's `block-pypi`, then frozen in-repo by exact version and SHA-256.
- The source task image receives an additive dependency layer; the verifier still sees the task's full filesystem.
- Offline `apt-get`, installer `curl`, and `uvx` adapters only satisfy the source verifier's fixed bootstrap commands. Their PATH and `UV_OFFLINE=1` are verifier-phase environment variables, not agent defaults.
- `[verifier].network_mode = "no-network"` is mandatory and fail-closed for scored runs. A laptop-only advisory variant may omit this provider assertion when Docker Desktop cannot switch a shared container between phase networks; it retains the offline verifier environment and must be paired with a direct `docker run --network none` attestation of the same prepared image.
- Metadata records the dependency lock, source-test bytes, base-image digest, prepared-layer content hash, and whether verifier network enforcement was `enforced` or `advisory`. Advisory outputs also set `offline_attestation_required: true`. The runner must additionally record the built image digest and direct-attestation result.

Generate the hello-world M1 task:

```bash
uv run python -m harbor_buzz_orchestra.prepare_verifier \
  /path/to/harbor/examples/tasks/hello-world \
  /tmp/prepared-hello-world
```

Generate the laptop advisory variant only when Docker Desktop rejects phase switching:

```bash
uv run python -m harbor_buzz_orchestra.prepare_verifier \
  /path/to/harbor/examples/tasks/hello-world \
  /tmp/prepared-hello-world-advisory \
  --network-enforcement advisory
```

The advisory task is suitable for laptop integration measurements, not a policy-enforced score. Before running it through Harbor, build that prepared image and run its unchanged verifier with `docker run --network none`; retain the image digest, reward, and test result beside the Harbor trial artifacts.

A prepared image is visible to the agent and is therefore richer than the stock TB-2.1 image. Internal comparisons remain valid only when every benchmark arm uses the same prepared task. Absolute rewards have an external-comparability caveat and must be reported with `prepared_image: true` plus the layer content hash.

## Execution environment

Laptop integration trials use the explicit advisory generator variant described above: Harbor runs the offline-configured verifier without claiming phase-network enforcement, while a direct `docker run --network none` attestation proves the same prepared image grades offline. Metadata and reports must call these laptop results advisory, never policy-enforced. Native Linux remains required when the benchmark needs Harbor itself to enforce verifier phase isolation.
