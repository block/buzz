# Offline Command Adviser acceptance record

## Candidate

- Phase: 5 — integrated disconnected operation
- Branch: `codex/phase-5-disconnected-acceptance`
- Pull request: [#27](https://github.com/NavigatorRAN/buzz/pull/27)
- Base: Phase 4 merge `87fe1a26c7e937933d82c2873b7e726b4ebff308`
- Phase 5.1 integration: PR #28 merge `201b19b76766c7fc4638f2e7f0aa6e11426a053b`
- Thread-context correction: `6cefe4b71b41e46b6750635fe75bb721bc8e49c4`
- Dependency audit refresh: `9cc044833c89a73694be37269cc78e0ddcc61bc3` and `aacc22ef5a6ebf0b7ad85e7c9f9f59b865c34ba4`
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
| Published candidate CI | Pass | Every required PR #27 check completed successfully on 18 August 2026, including both desktop integration shards, relay E2E, security, Rust lint, cross-compilation, and the macOS release build |

## Installed component preflight

| Component | State | Evidence |
| --- | --- | --- |
| Installed app | Pass for online preflight | v0.5.8, Developer ID `SR52Q9EJ76`, CDHash `61f47810edf301eb0960f6760a1693a7ed3e677f`; signed, installed, launched, and relay health verified 18 August 2026 |
| Relay | Pass | loopback health returned `ok` |
| LM Studio | Pass | exact local generation; `gemma4-26b-official`, 65,536 context, reasoning off, generation capacity one |
| Mac-local RAG | Pass for online preflight | snapshot identity and ADF Doctrine semantic result include document, location, and `point_id`; physical offline repeat remains required |
| Mac-local Memory | Pass | LaunchAgent on port 18006; MCP server `memory` v3.4.7 |
| Active skills | Pass | one verified learned projection exists |
| Bundle manifest | Pass | `1ac2d4363a02a4ab4b425930c25595bc6af8fbd44cd3d81e64140f4a88168767`; metadata-only inventory of the final installed app and local runtime, with no duplicate model payload |
| Online readiness | Pass as designed | final installed app, manifest, disk reserve, model, relay, RAG, Memory, and skills all passed; `ready:false` only because the external default route was correctly observed |

A fresh online readiness run on 18 August 2026 initially failed only the disk
reserve check. The preceding full repository verification had recreated 13 GiB
of disposable Cargo output in the main checkout and additional per-worktree
Rust targets. Cleaning only those reproducible `target` directories restored
212,287,397,888 free bytes against the required 209,659,449,344 bytes. The
same run then returned `components_ready:true`; no application data, model,
RAG snapshot, Memory state, recovery artefact, or source worktree was removed.

Phase 5.1 added global local/cloud routing and offline RAG prefetch. Physical
testing then exposed one runtime defect: the adviser requested an ID-only root
filter while reconstructing a thread. The relay correctly rejected the
potentially private agent-skill read with HTTP 403. The candidate now limits
the root filter to Buzz message kinds 9 and 40002, matching the reply filters.
The corrected candidate was rebuilt with both Rust lockfiles refreshed for the
compatible non-yanked `async-utility` and `spin` releases and patched `h2`
release, then signed, installed, launched, and re-inventoried successfully.

Encrypted backups and recovery material are at `/Users/matthewwarren/Command Adviser Backups/phase5-20260817`. The verified signed-app rollbacks are `Command-Adviser-PR26-rollback.zip` and `Command Adviser.before-thread-context-fix-20260818-192213.app` in that directory.

## Physical acceptance

These gates cannot be closed by repository tests.

The first owner-controlled isolation on 17 August 2026 proved every local
component ready, but exposed a false-negative macOS route classification
(`route_probe_failed`). The regression is fixed and the physical gate must be
repeated with the refreshed recovery pack.

Owner-controlled testing on 18 August 2026 then confirmed that, with Wi-Fi
disconnected, an adviser used the qualified local model and reached the
Mac-local RAG MCP server. It also exposed and reproduced the thread-context
HTTP 403 described above. This evidence closes the basic local model/RAG path,
but not the multi-adviser, Memory, recovery, restart, or soak gates.

A second owner-controlled disconnected run on 18 August 2026 returned
`ready:true`, `components_ready:true`, `disconnected_observed:true`, and
`no_external_gateway` while the signed app, exact model, relay, semantic RAG,
Memory MCP, active skill projection, manifest, and disk reserve all passed.
The owner completed the two- and three-adviser journeys, local doctrine/RAG
use, and local Memory write/readback, and reported no other failure. The owner
confirmed the disconnected product is usable.

The interrupted Daily Command Brief did not resume after Command Adviser was
quit and relaunched. The owner explicitly accepts this residual limitation for
Phase 5: an interrupted brief must be started again after relaunch. Automatic
resume is not represented as working and remains eligible for a later product
refinement; it does not reopen the accepted local model, RAG, Memory, or
multi-adviser paths.

- [x] external default route absent while loopback services remain available;
- [x] one-adviser installed-app journey reaches the local model and RAG through capacity one;
- [x] two- and three-adviser installed-app journeys complete through capacity one;
- [x] local RAG citation and local Memory write/readback are present;
- [x] interrupted Daily Command Brief behaviour is owner accepted with a recorded deviation: the run does not resume after app relaunch and must be started again;
- [ ] app, LM Studio, RAG, Memory, and relay restart canaries pass;
- [ ] cold Mac restart restores the exact runtime and pending work;
- [ ] eight-hour overnight soak passes with zero cloud attempts and bounded growth; and
- [x] owner confirms the installed disconnected product is usable.

## Acceptance decision

**IN PROGRESS.** The short disconnected user journey is accepted, including
the explicit interrupted-brief deviation. Phase 5 is not sea-going accepted
until the service-restart, cold-restart, and eight-hour soak gates pass.
Optional Phase 6 model refinement remains closed.
