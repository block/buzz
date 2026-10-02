#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
workflow="$repo_root/.github/workflows/docker.yml"
selector="$repo_root/scripts/select-qualified-ci-run.jq"
predicate_builder="$repo_root/scripts/create-deployment-eligibility-predicate.jq"

require_literal() {
  local needle=$1
  grep -Fq -- "$needle" "$workflow" || {
    echo "relay image workflow is missing required delivery contract: $needle" >&2
    exit 1
  }
}

require_literal "  qualify:"
require_literal "actions: read"
require_literal "actions/workflows/ci.yml/runs"
require_literal 'select-qualified-ci-run.jq'
require_literal "needs: [build, qualify]"
require_literal "https://buzz.block.xyz/attestations/deployment-eligibility/v1"
require_literal "actions/attest@1e69f48acb82d1966a394da916b4c1698aa569d6"
require_literal "if: matrix.variant == 'release'"
require_literal "BUZZ_SOURCE_SHA"
require_literal "BUZZ_BUILD_ID"
require_literal "BUZZ_BUILD_URL"
require_literal '- "deploy/charts/buzz/Chart.yaml"'
require_literal '- "scripts/create-deployment-eligibility-predicate.jq"'
require_literal '- "scripts/select-qualified-ci-run.jq"'
require_literal '- "scripts/test-relay-image-eligibility-workflow.sh"'

if grep -Fq "buzz-staging-dev" "$workflow"; then
  echo "canonical relay image workflow references the preview-only image package" >&2
  exit 1
fi

# The merge job reads scripts/create-deployment-eligibility-predicate.jq from the
# workspace, so it must check out the source first. Guard against the checkout being
# dropped from that job (the workspace is otherwise empty and jq exits non-zero).
merge_job=$(awk '/^  merge:/{f=1} f&&/^  [a-z][a-z_-]*:$/&&!/^  merge:/{exit} f' "$workflow")
grep -Fq "actions/checkout@df4cb1c069e1874edd31b4311f1884172cec0e10" <<<"$merge_job" || {
  echo "merge job must check out the source before building the eligibility predicate" >&2
  exit 1
}

select_run() {
  jq -r --arg source_sha aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa -f "$selector" | jq -r '.id // empty'
}

selected=$(select_run <<'JSON'
{"workflow_runs":[
  {"id":100,"run_attempt":1,"head_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","head_branch":"main","event":"push","status":"completed","conclusion":"failure"},
  {"id":101,"run_attempt":1,"head_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","head_branch":"main","event":"push","status":"completed","conclusion":"success"}
]}
JSON
)
[[ "$selected" == "101" ]] || {
  echo "latest successful same-SHA main run was not selected" >&2
  exit 1
}

selected=$(select_run <<'JSON'
{"workflow_runs":[
  {"id":100,"run_attempt":1,"head_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","head_branch":"main","event":"push","status":"completed","conclusion":"success"},
  {"id":101,"run_attempt":1,"head_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","head_branch":"main","event":"push","status":"completed","conclusion":"failure"}
]}
JSON
)
[[ -z "$selected" ]] || {
  echo "stale successful run remained eligible after a newer failure" >&2
  exit 1
}

selected=$(select_run <<'JSON'
{"workflow_runs":[
  {"id":102,"run_attempt":1,"head_sha":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","head_branch":"main","event":"push","status":"completed","conclusion":"success"},
  {"id":103,"run_attempt":1,"head_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","head_branch":"feature","event":"pull_request","status":"completed","conclusion":"success"}
]}
JSON
)
[[ -z "$selected" ]] || {
  echo "wrong-SHA or pull-request CI run was accepted" >&2
  exit 1
}

predicate=$(jq -n \
  --arg source_repository "block/buzz" \
  --arg source_ref "refs/heads/main" \
  --arg source_sha "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" \
  --arg build_workflow ".github/workflows/docker.yml" \
  --argjson build_run_id 200 \
  --argjson build_run_attempt 2 \
  --arg build_run_url "https://github.com/block/buzz/actions/runs/200/attempts/2" \
  --arg qualification_workflow ".github/workflows/ci.yml" \
  --argjson qualification_run_id 201 \
  --argjson qualification_run_attempt 1 \
  --arg qualification_run_url "https://github.com/block/buzz/actions/runs/201" \
  --arg chart_version "0.1.8" \
  -f "$predicate_builder")

jq -e '
  .predicate_version == 1 and
  .eligible == true and
  .source.sha == "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" and
  .build.run_id == 200 and
  .qualification.run_id == 201 and
  .qualification.conclusion == "success" and
  .helm_chart == {"name":"buzz","compatible_version":"0.1.8"} and
  (has("schema") | not) and
  (.helm_chart | has("schema_compatibility") | not)
' <<<"$predicate" >/dev/null || {
  echo "deployment eligibility predicate has an invalid contract" >&2
  exit 1
}

echo "relay image eligibility workflow contract passed"
