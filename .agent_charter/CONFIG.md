# Buzz Agent Charter Configuration

```yaml
charter_version: 2
setup_complete: true
setup_completed_at: 2026-08-29T14:44:50Z
repo_name: buzz
repository:
  canonical: https://github.com/block/buzz
  main_branch: main
  worktree_root: /Users/howard/Developer/buzz-inline-thread-replies
openproject:
  authority: https://openproject.tail21f530.ts.net
  project: https://openproject.tail21f530.ts.net/projects/buzz
  wiki: https://openproject.tail21f530.ts.net/projects/buzz/wiki
  bootstrap: https://openproject.tail21f530.ts.net/projects/buzz/wiki/Bootstrap
  evidence_log: https://openproject.tail21f530.ts.net/projects/buzz/wiki/Evidence_Log
  assumptions_log: https://openproject.tail21f530.ts.net/projects/buzz/wiki/Assumptions_Log
discussion:
  authority: Buzz
  relay: wss://asymetryk-hive.tail21f530.ts.net
  channel_name: buzz
  channel_id: 1d057f17-9fb3-4e2a-8590-3ed915640fc0
  channel_url: buzz://channel/1d057f17-9fb3-4e2a-8590-3ed915640fc0
  thread_policy: one thread per OpenProject work package
  current_thread: buzz://message?channel=1d057f17-9fb3-4e2a-8590-3ed915640fc0&id=eefa69e730937120fb4c7d3c0bf084feeeba79a43411ff410073915cc59c2c3f
design:
  primary_viewport: 1280x720
  browser: Chrome
  human_north_star_approval_required: true
  current_north_star_decision: Howard approved direct implementation in the existing Buzz visual language
uat:
  environment: local desktop E2E mock bridge
  build: . ./bin/activate-hermit && cd desktop && pnpm build:e2e
  localhost: true
  instance_policy: spin_new
production:
  target: signed desktop release installed by Howard
  promote: isolated worktree -> focused checks -> desktop E2E and screenshots -> PR and review -> release gate
  rollback: revert the merged PR and ship the next signed desktop release
routing:
  parent_owns: [plan, implementation, review, merge, definition_of_done]
  bulk_delegation: false
  image_generation: omniroute-imagegen
validation:
  full_gate: . ./bin/activate-hermit && just ci
  focused: desktop unit tests, desktop TypeScript checks, desktop E2E mock bridge, rendered screenshots
  graft_required: true
git:
  branch_pattern: feature/<short-description>
  merge: squash-via-pr
  dco: git commit -s
  commit_policy: feature-bug-pr workflow with focused commits and normal review gates
secrets:
  references: [.env files, desktop OS keyring]
  rule: never copy secret values into repository files, logs, screenshots, or chat
self_improvement:
  on_commit: true
circuit_breaker:
  repeated_failure_limit: 2
```

## Mismatch audit

| Contract | Result | Evidence |
| --- | --- | --- |
| Repository instructions | Pass | `AGENTS.md` exists and remains authoritative for product and stack facts. |
| OpenProject authority | Pass | Project `Buzz` is reachable at project id 53. |
| Project wiki surfaces | Pass | Bootstrap, Evidence Log, and Assumptions Log exist. |
| Buzz authority | Pass | Private `buzz` channel exists and the current work package has a top-level thread. |
| Browser | Pass | Chrome is the required browser validation surface. |
| North Star routing | Pass | `omniroute-imagegen` is the only still-image route; Howard approved direct implementation for this change. |
| Viewport | Pass | `1280x720` matches `just desktop-screenshot` defaults. |
| UAT | Pass | Desktop mock bridge uses `pnpm build:e2e`; no plain browser build. |
| Graft | Pass | Repository is indexed and `graft_required` is enabled. |
| Definition of done | Pass | Existing `just ci` gate is preserved. |
| Commit policy | Pass | Existing PR workflow and required DCO sign-off are preserved. |
