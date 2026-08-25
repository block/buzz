# Pi ACP production canary — 2026-08-25

## Scope

One approved low-risk identity, **Caliper — AI Quality Engineer**, was switched from managed Codex ACP to `pi-acp` on the Linza production relay. No other managed-agent identity changed runtime. Fleet rollout remains unapproved.

Candidate source:

- Buzz PR head before this evidence commit: `4babbdb90184601ce8b7e764de82b86006aa2f74`;
- local packaged runtime also retained the reviewed efficiency commits from PR #6692;
- Pi: `0.84.2`;
- `pi-acp`: `0.1.0`;
- adapter install was a packed immutable npm artifact, not a worktree symlink.

The production app and managed-agent configuration were snapshotted before mutation. The installed app remained ad-hoc signed and passed strict deep signature verification after replacing its reviewed `buzz-acp` and `buzz` sidecars.

## Initial Pi canary

Inbound event:

- event: `f83650cac4536d3398f1c32101c3d4b675c15f16a711af169000f8c25d51e2c6`;
- expected exact response: `PI_CANARY_OK_1787648609`.

Observed:

- one exact reply from Caliper: `2076c5df81d9d7a6b39e204b28ea2bf61ade58e0e9444f01e6342431e61b5e59`;
- relay timestamps: 8 seconds from inbound event to reply;
- polling wall clock: 11 seconds;
- correct channel and reply tag;
- durable reservation and receipt stored under the identity-scoped local receipt directory;
- no duplicate reply.

## Exercised rollback

Caliper was stopped, restored to managed Codex ACP, restarted, and tested before Pi was re-enabled.

- rollback inbound: `21067389ab422f72de8a51997b3e167358762593b98257fc5dabfceca8cbe34c`;
- exact Codex reply: `f2a3bcb4d7bd8ef02d7b0ba7fd93004875e7e6e997fd6c523f0159af8c06ac1b`;
- wall-clock completion: 22 seconds;
- result: PASS.

No Pi and Codex adapter ran concurrently for the Caliper identity on the tested relay.

## Final production state

Caliper was switched back to `pi-acp`, restarted, and tested again.

- final inbound: `b991c117b2153cdc383d028e7b432cb28b3c28c574f6734f79f3123d181216d1`;
- exact Pi reply: `6da69304f064cc2d617b86369bf7919e209030615d6aada22cba52fe0f0f1b44`;
- wall-clock completion: 11 seconds;
- result: PASS.

Caliper remains the only approved Pi production canary. Every other Linza specialist remains on managed Codex ACP. Expansion requires a separate owner decision and fleet-level review.
