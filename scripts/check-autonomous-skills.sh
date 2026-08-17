#!/usr/bin/env bash
set -euo pipefail

cargo test -p buzz-core agent_skill
cargo test -p buzz-relay agent_skill
cargo test -p buzz-search p_gated
cargo test -p buzz-acp skill_
cargo test -p buzz-acp experience_
cargo test -p buzz-agent hints

echo "all autonomous-skill checks passed"
