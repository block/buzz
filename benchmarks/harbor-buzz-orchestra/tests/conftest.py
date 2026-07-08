from typing import Any
import pytest


@pytest.fixture
def manifest_data() -> dict[str, Any]:
    digest = "a" * 64
    generation = {
        "temperature": 0,
        "max_output_tokens": 4096,
        "context_window_tokens": 200000,
    }
    return {
        "schema_version": "1",
        "condition": "O",
        "roster": [
            {
                "id": "opus-orchestrator",
                "kind": "orchestrator",
                "role": "orchestrator",
                "count": 1,
                "endpoint": "databricks/opus",
                "model_revision": "2026-07-01",
                "prompt": {"path": "prompts/orchestrator.md", "sha256": digest},
                "generation": generation,
            },
            {
                "id": "qwen-workers",
                "kind": "worker",
                "role": "fast-worker",
                "count": 4,
                "concurrency": 2,
                "endpoint": "databricks/qwen",
                "model_revision": "revision-42",
                "prompt": {"path": "prompts/qwen.md", "sha256": digest},
                "generation": {**generation, "context_window_tokens": 32768},
            },
        ],
        "prices": {
            "databricks/opus": {
                "input_per_million_usd": 5,
                "cached_input_per_million_usd": 0.5,
                "output_per_million_usd": 25,
            },
            "databricks/qwen": {
                "input_per_million_usd": 0.1,
                "cached_input_per_million_usd": 0.01,
                "output_per_million_usd": 0.2,
            },
        },
        "trial_budget": {"timeout_seconds": 3600, "max_cost_usd": 5},
    }
