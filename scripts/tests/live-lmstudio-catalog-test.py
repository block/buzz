#!/usr/bin/env python3
"""Regression tests for acceptance canaries sharing LM Studio with RAG."""

from __future__ import annotations

import importlib.util
import io
import json
import pathlib
import unittest
from unittest import mock


SCRIPTS = pathlib.Path(__file__).resolve().parents[1]


def load_script(name: str):
    path = SCRIPTS / name
    spec = importlib.util.spec_from_file_location(path.stem.replace("-", "_"), path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def catalogue() -> dict:
    return {
        "models": [
            {
                "type": "embedding",
                "key": "text-embedding-bge-m3",
                "loaded_instances": [
                    {
                        "id": "bge-m3-offline",
                        "config": {"context_length": 8192},
                    }
                ],
            },
            {
                "type": "llm",
                "key": "google/gemma-4-26b-a4b",
                "loaded_instances": [
                    {
                        "id": "gemma4-26b-official",
                        "config": {"context_length": 65536, "parallel": 1},
                    }
                ],
            },
            {
                "type": "llm",
                "key": "qwen/qwen3.6-27b",
                "loaded_instances": [],
            },
        ]
    }


class JsonResponse(io.BytesIO):
    def __enter__(self):
        return self

    def __exit__(self, *_args):
        self.close()


class LiveCatalogueTests(unittest.TestCase):
    def test_queue_canary_allows_loaded_embedding_alongside_single_llm(self) -> None:
        module = load_script("live-lmstudio-serial-queue-canary.py")
        module.validate_catalog(catalogue(), "gemma4-26b-official")

    def test_tool_canary_allows_loaded_embedding_alongside_single_llm(self) -> None:
        module = load_script("live-lmstudio-tool-call-canary.py")
        module.validate_catalog(catalogue(), "gemma4-26b-official")

    def test_adapter_canary_allows_loaded_embedding_alongside_single_llm(self) -> None:
        module = load_script("live-lmstudio-adapter-canary.py")
        response = JsonResponse(json.dumps(catalogue()).encode("utf-8"))
        with mock.patch.object(module.urllib.request, "urlopen", return_value=response):
            module.validate_catalog()


if __name__ == "__main__":
    unittest.main()
