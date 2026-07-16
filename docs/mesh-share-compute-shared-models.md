# Share compute: recommended solo models + shared (split) models

This adds a **shared model** path to Settings → Compute → **Share compute**,
alongside the existing solo-serving flow, and restructures the card into a
**Recommended** section (the common path) and an **Advanced** section (power
users + shared models).

## Why

Today Share compute only serves *whole* models that fit on one machine. But
mesh-llm can run a single model **split across several members** ("Skippy
splits" / staged layer-range distributed inference), which is the only way to
host models too large for any single box (Qwen3-235B, DeepSeek-V3.2, …).

The mesh runtime already does the hard part:

- **Implicit split.** When a model doesn't fit locally (fails the
  `local_capacity < model_bytes × 1.10` check), the runtime chooses the split
  path automatically — no explicit flag needed.
- **Waits for a quorum.** The serve startup loop treats "not enough
  participants" as a *durable waiting state*, not a failure: it emits
  "Split waiting for peers" and retries on membership changes until ≥2 eligible
  members hosting the same model form a quorum, then splits and serves.

So Buzz's job is just to **set the table**: offer splittable models and reflect
the waiting/running state. The mesh decides whether there's enough capacity and
does the splitting itself.

## What this PR does

### Recommended (solo) — the common path
The curated, hardware-ranked solo picker is now the default, front-and-center
"Recommended model" section. Unchanged behavior: pick a model that fits this
machine and share it.

### Advanced
Collapsed by default. Contains:

1. **Serve a model solo** — the free-text model field + "already installed"
   picklist (moved here from the top).
2. **Join a shared model** — a curated list of `meshllm/…-layers` layer
   packages, smallest → largest, each too big for one machine. Rows show size
   and a rough `~N members` estimate for this hardware, plus an honest
   up-front warning: *"They run slower than a single-machine model: the group
   trades speed for size."*
3. **Max VRAM (GB)** — reframed: for shared models a lower cap means you host a
   smaller slice.

### Cohort-aware status (coarse, by design)
The toggle's status line is now shared-model-aware, built entirely on the
**existing** SDK status (`state` / `health`) — no upstream mesh-llm change:

| State | Solo copy | Shared copy |
|---|---|---|
| `starting` | "Starting…" | **"Waiting for more members to join before this shared model can run."** |
| `running` | "Sharing {model} with relay members." | **"Running {model} with the group. You're hosting part of it."** |

This is deliberately coarse — we can say *"needs more members"* vs *"running"*,
but not yet *"3 of 4 members"*, because the SDK's `MeshNodeStatus` does not yet
expose a participant count. That richer "X of N ready" progress is a **future
enhancement** gated on an upstream mesh-llm status field
(`WaitingForParticipants { have, need }`); it is intentionally out of scope
here so the v1 ships on today's SDK.

## How a shared model actually runs (no code changes needed to the SDK)

1. A member picks a `meshllm/…-layers` model in Advanced → Join a shared model.
2. Buzz calls `mesh_start_node` with that ref (same path as solo).
3. It's too big to fit locally → the mesh runtime picks the split path.
4. The node advertises interest and **waits** (status shows "Waiting for more
   members…").
5. As other members pick the same model in the same community mesh, capacity
   accumulates; once ≥2 eligible members are present, the mesh plans the
   topology, assigns layer ranges, and serves. Status flips to "Running … with
   the group."

## Scope / honesty notes

- **No mesh-llm changes.** Everything works against the pinned `v0.73.1` SDK.
- **No forced split.** Buzz relies on *implicit* split (model too large) — it
  never forces a split of a model that would fit one box (the SDK has no such
  flag). This matches the "host something too big for one machine" use case.
- **Curated, not exhaustive.** The shared list is a hand-picked subset (one
  pick per size tier) for legibility, not the full ~79-entry `meshllm/…-layers`
  catalog.
- **Latency is real.** Split models add a network hop per token per stage; the
  UI warns about this before the user commits.

## Files

- `desktop/src-tauri/src/mesh_llm/catalog.rs` — `SharedModel` list,
  `estimate_members`, `shared`/`estimated_members` fields, `shared` list on the
  catalog. New tests.
- `desktop/src/shared/api/tauriMesh.ts` — `MeshCatalogEntry.shared` /
  `.estimatedMembers`, `MeshModelCatalog.shared`.
- `desktop/src/features/mesh-compute/ui/MeshComputeSettingsCard.tsx` —
  Recommended + Advanced restructure, `SharedModelPicker`, cohort-aware
  `StatusLine`.
- `desktop/src/features/mesh-compute/sharedModel.ts` (+ test) — `isSharedModelRef`,
  `sharedModelShortName`.
- `desktop/src/testing/e2eBridge.ts` — mock `mesh_model_catalog` (incl. `shared`).

## Tests

- Rust: `mesh_llm::catalog` (7 tests) — shared list present/ordered/flagged,
  member estimation, solo entries never shared.
- TS: `sharedModel.test.mjs` (9 tests) — shared detection + short-name.
- `just ci` gates: biome, tsc, frontend build, rust fmt + clippy — all green.
