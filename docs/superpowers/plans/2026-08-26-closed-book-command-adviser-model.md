# Closed-Book Command Adviser Model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build, train, evaluate, and package `command-adviser-qwen3.8-27b-v1` so Hermes Agent can use the complete RAG knowledge corpus through LM Studio without a runtime RAG MCP dependency.

**Architecture:** A private `Command Adviser Model Lab` repository owns read-only Qdrant export, corpus reconstruction, dataset generation, checkpointed BF16 LoRA training, closed-book evaluation, and GGUF packaging. VM 102 remains the authoritative source corpus; all large artefacts and compute live under `/home/matt/command-adviser-model-lab` on the DGX Spark; the MacBook receives only an accepted release bundle.

**Tech Stack:** Python 3.12, Pydantic 2, HTTPX, pytest, Hugging Face Transformers/PEFT/Accelerate, PyTorch CUDA, Docker with the NVIDIA runtime, Qwen/Qwen3.8-27B, llama.cpp GGUF tooling, LM Studio, Hermes Agent.

**Spec:** `docs/superpowers/specs/2026-08-26-closed-book-command-adviser-model-design.md`

## Global Constraints

- Pin the base model to `Qwen/Qwen3.8-27B` revision `1d4bf0f2ff6012fd82039f2fa52739d0dd7c60c0`.
- Treat Qdrant at `http://192.168.1.107:6333` as read-only; no exporter operation may create snapshots, update payloads, or delete points.
- Reconcile the production `documents` collection against an exact point count before and after every export. The approved starting inventory is 123,593 production points; `smoke_test` is not training data.
- Interpret payload field `collection` as the logical knowledge collection. The Qdrant collection name `documents` is only the physical container.
- Include every valid production point with non-empty `text`; record invalid points and fail instead of silently omitting them.
- Run export, reconstruction, training, evaluation, merge, and quantisation on the DGX Spark.
- Use `/home/matt/command-adviser-model-lab` because it is writable without owner intervention; do not require `/opt` or sudo.
- Keep vision modules frozen during text-only CPT and SFT.
- Start with BF16 LoRA, gradient checkpointing, sequence length 2,048, micro-batch 1, and gradient accumulation 16. Do not depend on 4-bit training support on ARM64.
- Keep every run immutable under a unique run ID and resume only from a validated checkpoint belonging to that run.
- Evaluate the base and candidate with RAG, Memory MCP, source documents, and internet access unavailable to the answering process.
- Do not change the MacBook's default Hermes model until the candidate passes all declared gates.
- Use ordinary private-repository and checksum hygiene; do not add per-document approvals or classification bureaucracy.
- Use a separate PR for each implementation phase and signed commits in Buzz.

---

## File Map

The new private repository `/Users/matthewwarren/Documents/Command Adviser Model Lab` contains:

```text
pyproject.toml                         package and test dependencies
README.md                              operator entry points and current phase
configs/base-model.json                immutable base-model identity
configs/trial.json                     trial sampling and training parameters
configs/full.json                      full-run sampling and training parameters
configs/gates.json                     predeclared evaluation thresholds
docker/Dockerfile                      pinned Spark training image
src/command_adviser_model_lab/
  __init__.py                          package version
  cli.py                               command-line routing
  hashing.py                           canonical JSON and streaming SHA-256
  schema.py                            corpus, manifest, dataset, and run models
  qdrant.py                            read-only scrolling client
  export_corpus.py                     atomic reconciled corpus export
  reconstruct.py                       document ordering and overlap removal
  duplicates.py                        exact and measured near-duplicate report
  dataset.py                           deterministic CPT/SFT/evaluation splits
  generate_instructions.py             local source-backed instruction generation
  status.py                            atomic run status and checkpoint journal
  model.py                             base-model load and text-module selection
  train_cpt.py                         continued pre-training LoRA stage
  train_sft.py                         knowledge and Hermes SFT stage
  evaluate.py                          closed-book base/candidate comparison
  package.py                           merge, GGUF conversion, and release manifest
tests/                                 one focused test module per source module
scripts/spark-sync.sh                  reviewed-code sync to Spark
scripts/spark-run.sh                   detached checkpointed job launcher
scripts/spark-status.sh                read-only status command
```

Buzz changes only after a model package passes Spark evaluation:

```text
docs/command-console/closed-book-model-acceptance.md
```

## Task 1: Create the Private Model Lab Repository and Package Skeleton

**Files:**

- Create repository: `/Users/matthewwarren/Documents/Command Adviser Model Lab`
- Create: `pyproject.toml`
- Create: `src/command_adviser_model_lab/__init__.py`
- Create: `src/command_adviser_model_lab/cli.py`
- Create: `tests/test_cli.py`
- Create: `.gitignore`
- Create: `README.md`

**Interfaces:**

- Consumes: no earlier implementation task
- Produces: `command-adviser-model` console script and importable package

- [ ] **Step 1: Create the private GitHub repository and clone it**

Run:

```bash
gh repo create NavigatorRAN/command-adviser-model-lab \
  --private \
  --description "Closed-book Command Adviser model training and evaluation" \
  --clone=false
git clone https://github.com/NavigatorRAN/command-adviser-model-lab.git \
  "/Users/matthewwarren/Documents/Command Adviser Model Lab"
git -C "/Users/matthewwarren/Documents/Command Adviser Model Lab" \
  switch -c codex/phase-1-model-lab-foundation
```

Expected: an empty private repository on the implementation branch.

- [ ] **Step 2: Write the failing CLI test**

```python
from command_adviser_model_lab.cli import main


def test_version_command(capsys):
    assert main(["version"]) == 0
    assert capsys.readouterr().out == "command-adviser-model-lab 0.1.0\n"
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `python3 -m pytest tests/test_cli.py -q`

Expected: FAIL because the package does not exist.

- [ ] **Step 4: Add the minimal package and pinned development dependencies**

`pyproject.toml` must define Python `>=3.12`, Hatchling, the console script
`command-adviser-model = command_adviser_model_lab.cli:console`, and these
initial dependencies:

```toml
dependencies = [
  "httpx==0.28.1",
  "pydantic==2.12.5",
]

[project.optional-dependencies]
dev = ["pytest==9.0.2", "pytest-cov==7.0.0", "ruff==0.14.14"]
```

`cli.py` must implement `main(argv: Sequence[str] | None = None) -> int` and a
`console() -> None` wrapper that exits with the returned code.

- [ ] **Step 5: Run the package gate**

Run:

```bash
python3 -m venv .venv
.venv/bin/pip install -e '.[dev]'
.venv/bin/pytest -q
.venv/bin/ruff check .
```

Expected: one passing test and no lint errors.

- [ ] **Step 6: Commit, push, and open the first Model Lab PR**

```bash
git add .
git commit -m "chore: scaffold command adviser model lab"
git push -u origin codex/phase-1-model-lab-foundation
gh pr create --draft --base main \
  --title "Phase 1: scaffold Command Adviser Model Lab" \
  --body "Creates the private training package and executable test gate."
```

## Task 2: Define Canonical Corpus Records and a Read-Only Qdrant Client

**Files:**

- Create: `src/command_adviser_model_lab/schema.py`
- Create: `src/command_adviser_model_lab/qdrant.py`
- Create: `src/command_adviser_model_lab/hashing.py`
- Create: `tests/test_schema.py`
- Create: `tests/test_qdrant.py`

**Interfaces:**

- Consumes: HTTP endpoint and physical Qdrant collection name
- Produces: `CorpusRecord`, `CorpusManifest`, and
  `QdrantReader.iter_points(collection, page_size)`

- [ ] **Step 1: Write schema tests for the live payload shape**

Use a fixture containing `point_id`, `doc_id`, `chunk_idx`, `text`,
`chunk_type`, `section_path`, `page_no`, `pages`, `doc_path`, `doc_name`,
logical `collection`, tags, ingest/model/parser versions, refs, `doc_type`,
language, `content_hash`, and `chunk_profile`. Assert that missing or blank
`text`, `doc_id`, `doc_name`, or logical `collection` fails validation.

```python
def test_corpus_record_keeps_identity(live_payload):
    record = CorpusRecord.from_qdrant("point-1", live_payload)
    assert record.point_id == "point-1"
    assert record.logical_collection == "ADF Doctrine"
    assert record.chunk_idx == 241
```

- [ ] **Step 2: Write read-only scrolling tests with `httpx.MockTransport`**

The mock must return two pages and assert every request is `POST` to
`/collections/documents/points/scroll` with `with_payload: true` and
`with_vector: false`. Also assert the client rejects any endpoint path,
redirect, non-HTTP scheme, duplicate JSON key, page larger than the configured
limit, or missing `next_page_offset` progression.

- [ ] **Step 3: Run focused tests and confirm failure**

Run: `.venv/bin/pytest tests/test_schema.py tests/test_qdrant.py -q`

Expected: FAIL on missing modules.

- [ ] **Step 4: Implement the schemas and client**

The reader interface is exact:

```python
class QdrantReader:
    def exact_count(self, collection: str) -> int: ...
    def iter_points(
        self, collection: str, *, page_size: int = 256
    ) -> Iterator[CorpusRecord]: ...
```

The client must use `follow_redirects=False`, `trust_env=False`, bounded JSON
responses, a 60-second timeout, and only the count and scroll endpoints. The
canonical JSON helper serialises with sorted keys, UTF-8, and compact
separators.

- [ ] **Step 5: Run the focused and complete local gates**

Run:

```bash
.venv/bin/pytest tests/test_schema.py tests/test_qdrant.py -q
.venv/bin/pytest -q
.venv/bin/ruff check .
```

Expected: all tests pass.

- [ ] **Step 6: Commit the client**

```bash
git add src tests
git commit -m "feat: add read-only qdrant corpus client"
```

## Task 3: Export and Reconcile the Complete Production Corpus

**Files:**

- Create: `src/command_adviser_model_lab/export_corpus.py`
- Modify: `src/command_adviser_model_lab/cli.py`
- Create: `tests/test_export_corpus.py`

**Interfaces:**

- Consumes: `QdrantReader.iter_points("documents")`
- Produces: `exports/<export_id>/corpus.jsonl.gz` and `manifest.json`

- [ ] **Step 1: Write failure-first export tests**

Cover an exact three-record export, interrupted temporary output, duplicate
point ID, invalid payload, before/after count drift, emitted-record mismatch,
logical collection counts, document counts, deterministic record order, file
hash, and atomic final directory activation.

```python
def test_export_reconciles_before_after_and_written_counts(tmp_path, reader):
    result = export_corpus(reader, tmp_path, physical_collection="documents")
    manifest = json.loads((result / "manifest.json").read_text())
    assert manifest["source_point_count_before"] == 3
    assert manifest["source_point_count_after"] == 3
    assert manifest["written_point_count"] == 3
```

- [ ] **Step 2: Run the focused test to verify failure**

Run: `.venv/bin/pytest tests/test_export_corpus.py -q`

Expected: FAIL because `export_corpus` is missing.

- [ ] **Step 3: Implement streaming atomic export**

Write newline-delimited canonical `CorpusRecord` JSON into gzip with
`mtime=0`. Calculate SHA-256 and byte count while streaming, count logical
collections and document IDs, and reject duplicate point IDs. Derive
`export_id` as `sha256:<manifest-input-hash>` rather than from the wall clock.
Write to `.staging-<uuid>` and rename only after the second exact count matches.

Add the exact command:

```text
command-adviser-model export \
  --qdrant-url http://192.168.1.107:6333 \
  --physical-collection documents \
  --output-root /home/matt/command-adviser-model-lab/data/exports
```

- [ ] **Step 4: Run focused tests and one bounded live sample**

Run:

```bash
.venv/bin/pytest tests/test_export_corpus.py -q
command-adviser-model export --qdrant-url http://192.168.1.107:6333 \
  --physical-collection documents --output-root /tmp/model-lab-export \
  --max-points 512
```

Expected: tests pass; bounded sample writes exactly 512 validated records and
marks the manifest `complete: false` so it cannot be used for full training.

- [ ] **Step 5: Deploy reviewed code to Spark and run the full export**

Use `scripts/spark-sync.sh` to sync the exact Git commit into
`/home/matt/command-adviser-model-lab/src`, then run the full command without
`--max-points`. Verify `written_point_count` equals the live before/after count
and the sum of `logical_collection_counts`.

- [ ] **Step 6: Commit the exporter and attach the manifest summary to the PR**

```bash
git add src tests
git commit -m "feat: export reconciled qdrant corpus"
git push
```

## Task 4: Reconstruct Documents and Report Duplicate Material

**Files:**

- Create: `src/command_adviser_model_lab/reconstruct.py`
- Create: `src/command_adviser_model_lab/duplicates.py`
- Modify: `src/command_adviser_model_lab/cli.py`
- Create: `tests/test_reconstruct.py`
- Create: `tests/test_duplicates.py`

**Interfaces:**

- Consumes: complete `corpus.jsonl.gz` and its manifest identity
- Produces: `documents.jsonl.gz`, `documents.manifest.json`, and
  `duplicates.jsonl.gz`

- [ ] **Step 1: Write reconstruction and overlap tests**

Test out-of-order chunks, repeated chunk index, exact suffix/prefix overlap,
short coincidental overlap, page transition, table/code preservation,
figure-only content, one `doc_id` appearing in two logical collections, and an
empty reconstructed document.

The overlap rule is exact: remove the longest equal suffix/prefix of at least
80 Unicode characters after newline normalisation; never fuzzy-delete content.

- [ ] **Step 2: Write duplicate-report tests**

Exact duplicates use the SHA-256 of normalised reconstructed text. Near
duplicates use 64-bit SimHash over lowercase word 3-grams and are reported when
Hamming distance is at most 3; they are not removed automatically. Assert that
exact duplicates retain one canonical training record plus all source
identities in `duplicate_sources`.

- [ ] **Step 3: Run the tests to verify failure**

Run: `.venv/bin/pytest tests/test_reconstruct.py tests/test_duplicates.py -q`

Expected: FAIL on missing modules.

- [ ] **Step 4: Implement disk-bounded reconstruction**

Load raw records into a temporary SQLite database keyed by
`(doc_id, logical_collection, chunk_idx, point_id)`. Stream ordered groups into
the output so the process does not hold the full corpus in memory. Preserve a
list of point IDs, source pages, chunk types, document metadata, and the source
export ID on every reconstructed record.

- [ ] **Step 5: Run the local gate and full Spark reconstruction**

Run:

```bash
.venv/bin/pytest -q
.venv/bin/ruff check .
command-adviser-model reconstruct \
  --export-manifest /home/matt/command-adviser-model-lab/data/exports/<export-id>/manifest.json \
  --output-root /home/matt/command-adviser-model-lab/data/reconstructed
```

Replace `<export-id>` in the actual command with the exact ID printed by Task 3;
the command itself rejects a partial or hash-mismatched export.

- [ ] **Step 6: Commit the reconstruction stage**

```bash
git add src tests
git commit -m "feat: reconstruct rag documents for training"
git push
```

## Task 5: Build Deterministic Trial, Full, and Hidden Evaluation Datasets

**Files:**

- Create: `src/command_adviser_model_lab/dataset.py`
- Create: `src/command_adviser_model_lab/generate_instructions.py`
- Create: `configs/trial.json`
- Create: `configs/full.json`
- Create: `tests/test_dataset.py`
- Create: `tests/test_generate_instructions.py`

**Interfaces:**

- Consumes: reconstructed documents and a local OpenAI-compatible generator
- Produces: immutable CPT, knowledge-SFT, Hermes-SFT, and evaluation JSONL files

- [ ] **Step 1: Write deterministic split tests**

Assert that every logical collection appears in the trial, document identities
never cross train/evaluation task-generation partitions, repeated runs are
byte-identical, full mode includes every canonical document, and collection
sampling is bounded without allowing the largest collection to erase smaller
ones.

`trial.json` uses:

```json
{
  "seed": 38027,
  "mode": "trial",
  "max_documents_per_collection": 256,
  "max_cpt_tokens": 50000000,
  "sequence_length": 2048,
  "evaluation_documents_per_collection": 12,
  "instruction_items_per_document": 3
}
```

`full.json` sets `mode` to `full`, both maximum fields to `null`, sequence
length to 2,048, evaluation documents per collection to 24, and instruction
items per document to 3.

- [ ] **Step 2: Write instruction-generation contract tests**

The generator must return strict JSON with `question`, `answer`, `task_family`,
`source_doc_ids`, and `source_excerpt_sha256`. Reject answers not supported by
the supplied source excerpt, duplicate questions, empty answers, raw thinking
tags, or tool calls. Generate direct recall, paraphrase, synthesis,
troubleshooting/coding where applicable, conflict/revision, and no-answer items.

- [ ] **Step 3: Implement CPT packing inputs and dataset manifests**

CPT JSONL records contain `text`, `doc_id`, and `logical_collection`; packing
happens in the trainer so no content crosses an evaluation audit boundary.
Every dataset manifest records source export, reconstruction, config, tokenizer,
generator model, generator parameters, item counts, token counts, and hashes.

- [ ] **Step 4: Implement resumable local instruction generation**

Use the Spark-local OpenAI-compatible endpoint and write one validated item at
a time to an append-only staging journal keyed by source hash and prompt
version. A restart skips only already validated keys. Finalisation sorts by key,
writes canonical JSONL, and atomically publishes the dataset.

- [ ] **Step 5: Run the trial dataset build and QA report**

Run:

```bash
command-adviser-model dataset build \
  --documents-manifest /home/matt/command-adviser-model-lab/data/reconstructed/<reconstruction-id>/documents.manifest.json \
  --config configs/trial.json \
  --output-root /home/matt/command-adviser-model-lab/data/datasets
```

The generated report must show non-zero CPT and evaluation counts for every
logical collection and zero invalid or unsupported final items.

- [ ] **Step 6: Commit dataset construction**

```bash
git add src tests configs
git commit -m "feat: build stratified model training datasets"
git push
```

## Task 6: Add Immutable Run State and Mac mini Monitoring

**Files:**

- Create: `src/command_adviser_model_lab/status.py`
- Create: `scripts/spark-sync.sh`
- Create: `scripts/spark-run.sh`
- Create: `scripts/spark-status.sh`
- Create: `tests/test_status.py`
- Create: `tests/test_spark_scripts.py`

**Interfaces:**

- Consumes: run configuration plus stage subprocess
- Produces: `runs/<run-id>/status.json`, event journal, logs, and checkpoint refs

- [ ] **Step 1: Write run-state tests**

Test `queued -> running -> completed`, `running -> failed`, interrupted restart,
monotonic sequence numbers, atomic status replacement, checkpoint hash
validation, mismatched run ID rejection, immutable terminal state, and redacted
command/environment reporting.

- [ ] **Step 2: Implement `RunStatusStore`**

Expose:

```python
class RunStatusStore:
    @classmethod
    def create(cls, root: Path, config: RunConfig) -> "RunStatusStore": ...
    def transition(self, state: RunState, *, message: str) -> RunStatus: ...
    def heartbeat(self, *, stage: str, step: int, loss: float | None) -> RunStatus: ...
    def checkpoint(self, path: Path, *, step: int) -> RunStatus: ...
```

Each write appends canonical JSON to `events.jsonl`, fsyncs it, then atomically
replaces `status.json`.

- [ ] **Step 3: Add exact Spark scripts**

`spark-sync.sh` resolves the current reviewed commit, creates
`/home/matt/command-adviser-model-lab/src`, and uses `rsync --delete` while
excluding `.git`, virtual environments, data, runs, and weights.

`spark-run.sh` uses `docker run --gpus all --ipc=host`, mounts the repository
read-only and data/runs/cache read-write, writes the container ID, and follows
the run's status contract. `spark-status.sh` prints the exact JSON over SSH and
never changes Spark state.

- [ ] **Step 4: Run shell and Python tests**

Run:

```bash
.venv/bin/pytest tests/test_status.py tests/test_spark_scripts.py -q
shellcheck scripts/*.sh
```

Expected: all tests pass and ShellCheck emits no findings.

- [ ] **Step 5: Commit run management**

```bash
git add src scripts tests
git commit -m "feat: add checkpointed spark run management"
git push
```

## Task 7: Qualify the Spark Training Image and Official Base Model

**Files:**

- Create: `docker/Dockerfile`
- Create: `configs/base-model.json`
- Create: `src/command_adviser_model_lab/model.py`
- Create: `tests/test_model.py`
- Modify: `README.md`

**Interfaces:**

- Consumes: official model revision and Spark NVIDIA Docker runtime
- Produces: pinned training image digest and a completed base-model smoke run

- [ ] **Step 1: Pin the model identity**

`configs/base-model.json` must contain:

```json
{
  "model_id": "Qwen/Qwen3.8-27B",
  "revision": "1d4bf0f2ff6012fd82039f2fa52739d0dd7c60c0",
  "architecture": "Qwen3_5ForConditionalGeneration",
  "license": "apache-2.0",
  "native_context": 262144,
  "freeze_vision": true
}
```

- [ ] **Step 2: Write model-selection tests**

Use dummy named modules to assert LoRA targets include language-model linear
layers and exclude `visual`, `vision`, `merger`, output head, and embeddings.
Assert the loader always passes the pinned revision, `torch_dtype=bfloat16`,
`attn_implementation=sdpa`, and never enables remote code.

- [ ] **Step 3: Build the pinned ARM64 CUDA image**

Base the image on `nvcr.io/nvidia/pytorch:26.03-py3`. Install a pinned
Transformers commit that contains `Qwen3_5ForConditionalGeneration`, plus
`accelerate`, `peft`, `datasets`, `safetensors`, `sentencepiece`, `pydantic`,
and the local package. Record the final image digest in `README.md` and the run
manifest; a mutable tag alone is not accepted.

- [ ] **Step 4: Download and hash the official BF16 base on Spark**

Use `huggingface-cli download` with the exact revision and
`HF_HOME=/home/matt/.cache/huggingface`. Record the snapshot directory and hash
of `model.safetensors.index.json`. Do not copy the Mac's MLX quantisation to the
Spark because it is not a trainable source checkpoint.

- [ ] **Step 5: Run the base-model smoke**

Inside the pinned image:

```text
load tokenizer and processor
load BF16 model with device_map=cuda
verify architecture and revision
verify vision modules are frozen
run one deterministic text generation
run one native-image generation input
attach LoRA to text modules only
run one forward/backward/optimizer step at sequence length 512
save adapter
reload adapter
merge adapter into a temporary checkpoint
```

Capture GPU/unified-memory telemetry before load, after load, after backward,
and after merge. Fail on OOM, NaN/Inf loss, missing adapter weights, or changed
vision parameters.

- [ ] **Step 6: Commit and merge Phase 1 only after the smoke passes**

Run the full Model Lab gate, push the results, mark the PR ready, and merge it.
If the image or official model cannot complete one optimiser step on the single
Spark, stop before corpus training and report the measured blocker.

## Task 8: Implement and Run the Continued Pre-Training Trial

**Files:**

- Create: `src/command_adviser_model_lab/train_cpt.py`
- Create: `tests/test_train_cpt.py`
- Modify: `configs/trial.json`

**Interfaces:**

- Consumes: immutable trial CPT dataset and pinned base model
- Produces: checkpointed CPT LoRA adapter and merged intermediate model

- [ ] **Step 1: Open Phase 2 PR and write trainer tests**

Test causal labels equal input IDs except padding `-100`, deterministic packing,
collection-balanced sampling, gradient accumulation, vision freeze, resume from
the latest validated checkpoint, no resume across run IDs, periodic status
heartbeats, and NaN/Inf abort.

- [ ] **Step 2: Implement BF16 LoRA CPT**

Use `Trainer`/`Accelerate`, gradient checkpointing, SDPA, BF16, sequence 2,048,
micro-batch 1, accumulation 16, LoRA rank 64, alpha 128, dropout 0.05, learning
rate `2e-5`, cosine schedule, 3% warmup, max gradient norm 1.0, checkpoint every
250 optimiser steps, and evaluation every 250 steps. Target only verified text
linear modules.

- [ ] **Step 3: Run the interruption/recovery canary**

Run 20 optimiser steps, terminate the container after a durable checkpoint,
restart the same run, and verify it resumes at the next step with identical
dataset and model identities.

- [ ] **Step 4: Run the stratified CPT trial**

Launch detached through `spark-run.sh`. The Mac mini or this Mac may poll
`spark-status.sh`; losing the monitor must not affect the job. Continue until
the declared trial token budget completes or the run enters failed state.

- [ ] **Step 5: Merge the CPT adapter and run base-regression smoke**

Verify deterministic text generation, strict JSON, one tool-call-shaped output,
and native image input before allowing knowledge SFT. Preserve both the adapter
and merged intermediate model.

- [ ] **Step 6: Commit CPT implementation and recorded trial configuration**

```bash
git add src tests configs README.md
git commit -m "feat: run checkpointed continued pretraining"
git push
```

## Task 9: Implement Knowledge and Hermes Instruction Tuning

**Files:**

- Create: `src/command_adviser_model_lab/train_sft.py`
- Create: `tests/test_train_sft.py`
- Modify: `configs/trial.json`

**Interfaces:**

- Consumes: merged CPT model, knowledge SFT, and Hermes SFT JSONL
- Produces: final trial adapter and merged candidate checkpoint

- [ ] **Step 1: Write chat-template and loss-mask tests**

Assert Qwen's pinned chat template is used, user/system/tool-schema tokens are
masked from loss, assistant answer and tool-call tokens contribute to loss,
thinking is disabled for direct-output examples, malformed tool JSON is
rejected, and source identity metadata is not inserted into the assistant
answer unless the example explicitly asks for it.

- [ ] **Step 2: Implement sequential knowledge then Hermes SFT**

Start from the merged CPT checkpoint. Use BF16 LoRA rank 64, alpha 128,
dropout 0.05, sequence 4,096 where measured memory permits and 2,048 otherwise,
micro-batch 1, accumulation 16, learning rate `1e-5`, cosine schedule, 3%
warmup, and the same checkpoint/status contract. Train knowledge items first,
then Hermes tool/collaboration items as a separately identifiable stage.

- [ ] **Step 3: Run SFT checkpoint/recovery and structured-output canaries**

Interrupt and resume one knowledge checkpoint and one Hermes checkpoint. After
each stage, run exact JSON and tool-schema cases against held-out prompts.

- [ ] **Step 4: Complete the trial SFT and merge candidate**

Write the final merged HF checkpoint under the run directory, hash all shards,
and record CPT adapter, CPT merged model, knowledge adapter, Hermes adapter, and
final merged identities.

- [ ] **Step 5: Commit SFT implementation**

```bash
git add src tests configs README.md
git commit -m "feat: add knowledge and hermes instruction tuning"
git push
```

## Task 10: Evaluate Base and Candidate Closed-Book

**Files:**

- Create: `src/command_adviser_model_lab/evaluate.py`
- Create: `configs/gates.json`
- Create: `tests/test_evaluate.py`
- Create: `docs/evaluation.md`

**Interfaces:**

- Consumes: hidden evaluation dataset, base checkpoint, candidate checkpoint
- Produces: comparable signed reports and `promote: true|false`

- [ ] **Step 1: Predeclare gates before scoring the candidate**

`configs/gates.json` must require:

```json
{
  "minimum_closed_book_absolute_gain": 0.15,
  "minimum_collection_score_ratio": 0.90,
  "maximum_general_capability_drop": 0.05,
  "required_exact_json_rate": 1.0,
  "required_tool_call_rate": 0.95,
  "required_native_image_rate": 0.90,
  "required_cancellation_recovery_rate": 1.0,
  "required_three_request_queue_rate": 1.0
}
```

Scores are fractions from 0 to 1. Any missing logical collection or critical
gate sets `promote` false.

- [ ] **Step 2: Write evaluator tests**

Test deterministic exact/substring/JSON graders, source-backed semantic grading,
per-collection aggregation, missing collection failure, base/candidate prompt
identity, answer-process environment denial of RAG/Memory/internet variables,
critical capability override, and immutable report hashes.

- [ ] **Step 3: Run the base evaluation first**

Use the exact hidden suite and record its hash before exposing candidate scores.
The answering container receives only model weights and evaluation prompts; it
does not mount source excerpts or corpus files and runs with Docker network
disabled.

- [ ] **Step 4: Run candidate evaluation and capability regression**

Evaluate direct recall, paraphrase, synthesis, troubleshooting, coding,
conflict/revision, and no-answer by logical collection. Then execute exact text,
strict JSON, reasoning-off, stateful continuation, structured tool calls,
native image, cancellation/recovery, three-request queue, and held-out general
coding tests.

- [ ] **Step 5: Apply gates without changing thresholds**

If `promote` is false, keep the candidate and reports but do not run the full
training configuration. Adjust data/training in a new run and repeat the trial.
If true, merge Phase 2 and proceed automatically to Task 11.

## Task 11: Run Full Training, Package GGUF, and Qualify on the MacBook

**Files:**

- Create: `src/command_adviser_model_lab/package.py`
- Create: `tests/test_package.py`
- Create: `docs/release.md`
- Create in Buzz: `docs/command-console/closed-book-model-acceptance.md`

**Interfaces:**

- Consumes: full dataset, passing trial configuration, final merged checkpoint
- Produces: `command-adviser-qwen3.8-27b-v1` release bundle

- [ ] **Step 1: Open Phase 3 PR and run the full dataset build**

Use `configs/full.json`, the same source export, and the passing trial's prompt
and training versions. Verify every canonical reconstructed document appears in
the CPT manifest and every logical collection appears in SFT and evaluation.

- [ ] **Step 2: Run full CPT and sequential SFT**

Use the trial-proven parameters and immutable run contract. Continue through
interruptions from the latest validated checkpoint. Do not alter learning rate,
LoRA shape, sampling, or gates within the run.

- [ ] **Step 3: Evaluate the full candidate**

Run the complete Task 10 suite. Package only when `promote` is true and all
model shard hashes are present.

- [ ] **Step 4: Convert and quantise on Spark**

Pin a llama.cpp commit whose converter recognises `Qwen3_5ForConditionalGeneration`.
Convert the merged HF checkpoint to BF16 GGUF, then quantise Q4_K_M. Run
`llama-quantize --allow-requantize` only on the newly converted BF16 source.
Preserve the vision projector required by Qwen3.8 and run text plus image
inference against the packaged files on Spark.

- [ ] **Step 5: Build the release manifest**

The release directory contains the GGUF, vision projector, tokenizer/chat
template material, base and training identities, corpus/dataset/run IDs,
evaluation reports, recommended LM Studio settings, Apache-2.0 notices, and
SHA-256 for every file. The release manifest itself is canonical JSON and
hashed.

- [ ] **Step 6: Download only the release bundle to the MacBook**

Use resumable checksum-verified transfer from Spark. Import it alongside
`qwen/qwen3.8-27b`; do not overwrite the current model or change Hermes yet.

- [ ] **Step 7: Run MacBook and physical-disconnection acceptance**

Qualify 32K at parallelism 1 first, then 64K only if memory/swap telemetry is
safe. Run exact text, strict JSON, reasoning-off, continuation, tool call,
native image, cancellation/recovery, three queued requests, real Hermes/Buzz
collaboration, restart, and Daily Command Brief. Repeat with RAG MCP unavailable
and external network physically disconnected.

- [ ] **Step 8: Switch Hermes only after acceptance**

Record the previous model identifier, switch Hermes to the accepted local
identifier, run one real Buzz turn, and verify one-step rollback to
`qwen/qwen3.8-27b`. Complete the Buzz acceptance record and Model Lab release
PRs with the exact evidence.

## Plan Self-Review

- Spec coverage: full export, reconstruction, all logical collections, CPT,
  knowledge/Hermes SFT, Spark-only compute, monitoring, closed-book evaluation,
  multimodal/tool regression, packaging, Mac qualification, rollback, refresh
  identities, and ordinary data handling all map to explicit tasks.
- Subsystem boundary: the first implementation is isolated in the new Model Lab
  repository. RAG remains read-only and unchanged. Buzz changes only for the
  final acceptance record after a package passes Spark evaluation.
- Type consistency: `CorpusRecord`, `CorpusManifest`, `QdrantReader`,
  `RunStatusStore`, export/reconstruction/dataset IDs, and release identities
  are defined before downstream consumption.
- Placeholder scan: angle-bracket export and reconstruction IDs in operator
  examples are runtime values printed by prior commands, not unspecified code
  or design work. No implementation step contains deferred error handling or an
  undefined test requirement.
