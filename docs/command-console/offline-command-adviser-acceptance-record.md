# Offline Command Adviser acceptance record

## Candidate

- Phase: 5 — integrated disconnected operation
- Branch: `codex/phase-5-disconnected-acceptance`
- Pull request: [#27](https://github.com/NavigatorRAN/buzz/pull/27)
- Base: Phase 4 merge `87fe1a26c7e937933d82c2873b7e726b4ebff308`
- Phase 5.1 integration: PR #28 merge `201b19b76766c7fc4638f2e7f0aa6e11426a053b`
- Qualified model: `gemma4-26b-official`
- RAG snapshot: `f88174b38ae3bca3c0339d0d0bb9dafdec2fbb2507503c1b11e830c4895b735d`

## Automated evidence

| Gate | State | Evidence |
| --- | --- | --- |
| New worktree baseline | Pass | `just test-unit`, 17 August 2026 |
| Sea-going manifest fixtures | Pass | 10 tests: deterministic content identity, protected config, materialisation capacity |
| Readiness fixtures | Pass | app/model/relay/RAG/Memory/skills/disk/network failure matrix |
| macOS offline route regression | Pass | accepts both a successful route probe without an external gateway and BSD `not in table`; unrelated probe failures remain blocking |
| Soak fixtures | Pass | 6 tests: resume, cloud attempt, stuck run, duplicate publication, service loss, disk growth |
| Live LM Studio catalogue regression | Pass | 3 tests permit the accepted embedding instance while requiring one loaded LLM |
| Existing recovery and scheduler tests | Pass | 7 scheduler and 9 audit/recovery tests; adaptive-memory and autonomous-skill gates passed |
| Thread-context query regression | Pass | 716 `buzz-acp` unit tests and strict clippy; root and reply filters are explicitly limited to message kinds 9 and 40002 so the private agent-skill guard cannot reject adviser thread recall |
| Qualified model exact/FIFO canaries | Pass | `GEMMA64 READY`; three requests generated in order 1, 2, 3 with no overlap or second LLM |
| Full repository gate | Pass | `just ci`, 17 August 2026 |

## Installed component preflight

| Component | State | Evidence |
| --- | --- | --- |
| Installed app | Pass for online preflight | v0.5.8, Developer ID `SR52Q9EJ76`, CDHash `94d3c97e2bcec8440d7b000719b878b95a704ead`; signed 18 August 2026 |
| Relay | Pass | loopback health returned `ok` |
| LM Studio | Pass | exact local generation; `gemma4-26b-official`, 65,536 context, reasoning off, generation capacity one |
| Mac-local RAG | Pass for online preflight | snapshot identity and ADF Doctrine semantic result include document, location, and `point_id`; physical offline repeat remains required |
| Mac-local Memory | Pass | LaunchAgent on port 18006; MCP server `memory` v3.4.7 |
| Active skills | Pass | one verified learned projection exists |
| Bundle manifest | Pass before final runtime rebuild | `5afbe7249988a9a57cee5da4f8e2a43166b8644052a7bb32ca38d2c170a1498f`; metadata-only inventory, no duplicate model payload. Refresh again after installing the thread-context correction. |
| Online readiness | Pass as designed | every component passed; `ready:false` only because the external default route was correctly observed |

Phase 5.1 added global local/cloud routing and offline RAG prefetch. Physical
testing then exposed one runtime defect: the adviser requested an ID-only root
filter while reconstructing a thread. The relay correctly rejected the
potentially private agent-skill read with HTTP 403. The candidate now limits
the root filter to Buzz message kinds 9 and 40002, matching the reply filters.
A final signed rebuild, install, and manifest refresh are therefore required.

Encrypted backups and recovery material are at `/Users/matthewwarren/Command Adviser Backups/phase5-20260817`. The verified signed-app rollback is `Command-Adviser-PR26-rollback.zip` in that directory.

## Physical acceptance

These gates are deliberately open. They cannot be closed by repository tests.

The first owner-controlled isolation on 17 August 2026 proved every local
component ready, but exposed a false-negative macOS route classification
(`route_probe_failed`). The regression is fixed and the physical gate must be
repeated with the refreshed recovery pack.

Owner-controlled testing on 18 August 2026 then confirmed that, with Wi-Fi
disconnected, an adviser used the qualified local model and reached the
Mac-local RAG MCP server. It also exposed and reproduced the thread-context
HTTP 403 described above. This evidence closes the basic local model/RAG path,
but not the multi-adviser, Memory, recovery, restart, or soak gates.

- [x] external default route absent while loopback services remain available;
- [x] one-adviser installed-app journey reaches the local model and RAG through capacity one;
- [ ] two- and three-adviser installed-app journeys complete through capacity one;
- [ ] local RAG citation and local Memory write/readback are present;
- [ ] interrupted Daily Command Brief resumes without duplicate publication;
- [ ] app, LM Studio, RAG, Memory, and relay restart canaries pass;
- [ ] cold Mac restart restores the exact runtime and pending work;
- [ ] eight-hour overnight soak passes with zero cloud attempts and bounded growth; and
- [ ] owner confirms the installed disconnected product is usable.

## Acceptance decision

**IN PROGRESS.** Phase 5 is not sea-going accepted until the physical gates above pass. Optional Phase 6 model refinement remains closed.
