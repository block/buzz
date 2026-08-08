# `mixed-opus-terra.json` — why this config exists

Notes for `mixed-opus-terra.json`. They live in a sibling file rather than a
`_comment` key inside the JSON because `write_provisioner_config()` in
`scripts/benchmark.py` iterates `endpoints.items()` and indexes
`entry["api_key_env"]` **without** an `isinstance` guard (unlike
`required_key_envs()`, which has one). Any non-dict value in that top-level
object therefore crashes the launch with `TypeError: string indices must be
integers` — after container setup, so it reads as a harness bug rather than a
config typo. **Do not add descriptive keys to any endpoint config.**

## What it is for

`tb-gt-opus-2terra.yaml` (cell **G1ot**) needs Claude Opus 5 as its lead and two
`gpt-5.6-terra` subordinates. Those two models are not available from the same
vendor:

| model | `databricks-live.json` | `openai-live.json` | `anthropic-live.json` |
|---|---|---|---|
| Claude Opus 5 | ✅ `databricks-claude-opus-5` | ✗ | ✗ (Sonnet 4.6 / Haiku 4.5 only) |
| GPT-5.6 Terra | ✗ | ✅ `gpt-5.6-terra` | ✗ |

So no single-vendor config can serve G1ot, and this merged file is the only way
to run it at all.

It is deliberately **minimal** — exactly the two endpoints G1ot names. A
manifest typo then fails loudly at resolution instead of quietly binding to some
other model that happens to be present.

## Credentials

It requires **both** secrets:

- `DATABRICKS_TOKEN` — for the Opus 5 lead
- `OPENAI_API_KEY` — for the Terra seats; `benchmark.py`'s `resolve_openai_key()`
  copies it into `OPENAI_COMPAT_API_KEY`, which is what the `openai` provider
  actually reads

Neither `sweep.sh` (Databricks only) nor `sweep-openai.sh` (OpenAI only) exports
both. Use **`sweep-mixed.sh`**.

## Interpretive caveat

G1ot is cross-provider by necessity, so `G1 → G1ot` moves seat model *and* seat
provider together, and `G1st → G1ot` moves lead model *and* lead provider
together. Neither is a one-variable comparison. The clean seat-model read is
`G1s → G1st` (both single-route OpenAI). See the header of
`manifests/tb-gt-opus-2terra.yaml`.

Note this is a difference of degree, not kind: every Opus-led cell in the study
(C3, G1, G2) is *already* two-route internally, because Opus does not speak
`/responses` and takes `/ai-gateway/anthropic/v1/messages` while its seats take
the Databricks `/responses` path. G1ot widens that from two paths inside one
vendor to two vendors — a second set of rate limits and a second failure mode,
not a new class of confound.
