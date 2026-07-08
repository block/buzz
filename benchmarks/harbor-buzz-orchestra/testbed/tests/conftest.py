"""Shared fixtures for testbed provisioning tests."""

from __future__ import annotations

import pytest
from harbor_buzz_orchestra.manifest import ExperimentManifest

PROMPT_REF = {"path": "personas/x.md", "sha256": "0" * 64}


@pytest.fixture()
def manifest() -> ExperimentManifest:
    """A minimal two-class manifest: one orchestrator, two workers."""
    return ExperimentManifest.load(
        {
            "condition": "O-test",
            "roster": [
                {
                    "id": "worker-glm",
                    "kind": "worker",
                    "role": "implementer",
                    "count": 2,
                    "endpoint": "databricks/glm",
                    "model_revision": "glm-5.2",
                    "prompt": PROMPT_REF,
                    "generation": {
                        "max_output_tokens": 4096,
                        "context_window_tokens": 128000,
                    },
                },
                {
                    "id": "orch-opus",
                    "kind": "orchestrator",
                    "role": "lead",
                    "count": 1,
                    "endpoint": "databricks/opus",
                    "model_revision": "opus-4.8",
                    "prompt": PROMPT_REF,
                    "generation": {
                        "max_output_tokens": 8192,
                        "context_window_tokens": 200000,
                    },
                },
            ],
            "prices": {
                "databricks/glm": {
                    "input_per_million_usd": 0.5,
                    "cached_input_per_million_usd": 0.1,
                    "output_per_million_usd": 1.5,
                },
                "databricks/opus": {
                    "input_per_million_usd": 15.0,
                    "cached_input_per_million_usd": 1.5,
                    "output_per_million_usd": 75.0,
                },
            },
            "trial_budget": {"timeout_seconds": 3600},
        }
    )
