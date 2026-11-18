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

# Main pushes may skip only paths Docker never sees. The push filter must be, in
# order: "**", then directory exclusions that .dockerignore also excludes, then
# the re-included image workflow. Any other entry, or another order, could stop
# a change that alters the published image from publishing it.
push_trigger=$(awk '/^  push:$/{f=1;next} f&&/^  [a-z_]+:/{exit} f' "$workflow")
push_paths=()
while IFS= read -r entry; do
  push_paths+=("$entry")
done < <(awk '/^    paths:$/{f=1;next} f&&/^[[:space:]]*(#.*)?$/{next} f&&/^      - /{print;next} f{exit}' <<<"$push_trigger")
fail_push_paths() {
  echo "main push path filter: $1" >&2
  exit 1
}
(( ${#push_paths[@]} >= 3 )) || fail_push_paths "expected \"**\", directory exclusions, then the workflow"
[[ "${push_paths[0]}" == '      - "**"' ]] || fail_push_paths 'first entry must be "**"'
last=$(( ${#push_paths[@]} - 1 ))
[[ "${push_paths[$last]}" == '      - ".github/workflows/docker.yml"' ]] ||
  fail_push_paths "last entry must re-include .github/workflows/docker.yml"
for entry in "${push_paths[@]:1:last-1}"; do
  [[ "$entry" =~ ^\ {6}-\ \"!([A-Za-z0-9._-]+)/\*\*\"$ ]] ||
    fail_push_paths "only \"!<dir>/**\" exclusions may sit between \"**\" and the workflow, found: ${entry#      - }"
  excluded=${BASH_REMATCH[1]}
  grep -Fxq "$excluded/" "$repo_root/.dockerignore" ||
    fail_push_paths "skips $excluded/, which .dockerignore keeps in the image build context"
done

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

for annotation in \
  'index:org.opencontainers.image.source=${GITHUB_SERVER_URL}/${GITHUB_REPOSITORY}' \
  'index:org.opencontainers.image.revision=${SOURCE_SHA}'; do
  grep -Fxq "            --annotation \"$annotation\"" <<<"$merge_job" || {
    echo "relay image index is missing its source annotation: $annotation" >&2
    exit 1
  }
done

# Staging auto-promotes the newest main-<ts>-<7> gateway image, so the gateway
# must publish that tag and only from a source the relay CI qualified. Match
# whole lines (-x) so a commented-out or otherwise inactive copy cannot satisfy
# the contract after the live line regresses.
gateway_merge_job=$(awk '/^  push-gateway-merge:/{f=1} f&&/^  [a-z][a-z_-]*:$/&&!/^  push-gateway-merge:/{exit} f' "$workflow")
grep -Fxq "    needs: [push-gateway-build, qualify]" <<<"$gateway_merge_job" || {
  echo "push gateway image must publish only after same-SHA CI qualification" >&2
  exit 1
}
grep -Fxq "            type=raw,value=main-{{commit_date 'YYYYMMDDHHmmss'}}-{{sha}},enable=\${{ github.event_name == 'push' && github.ref == 'refs/heads/main' }}" <<<"$gateway_merge_job" || {
  echo "push gateway image must publish the sortable main-<commit UTC>-<7> tag on main pushes" >&2
  exit 1
}

# A main push publishes a gateway image only when its build inputs changed
# since the current :main image's source, so staging does not roll out
# identical gateways. Both gateway build jobs wait for that check.
for job in push-gateway-build push-gateway-personal-dev; do
  job_block=$(awk -v job="  $job:" '$0==job{f=1;print;next} f&&/^  [a-z][a-z_-]*:$/{exit} f' "$workflow")
  grep -Fxq "    needs: push-gateway-inputs" <<<"$job_block" &&
    grep -Fxq "    if: needs.push-gateway-inputs.outputs.build == 'true'" <<<"$job_block" || {
    echo "$job must run only when the push gateway build inputs changed" >&2
    exit 1
  }
done
inputs_job=$(awk '/^  push-gateway-inputs:/{f=1} f&&/^  [a-z][a-z_-]*:$/&&!/^  push-gateway-inputs:/{exit} f' "$workflow")
grep -Fxq "          MAIN_PUSH: \${{ github.event_name == 'push' && github.ref == 'refs/heads/main' }}" <<<"$inputs_job" &&
  grep -Fxq '        run: scripts/push-gateway-image-inputs-changed.sh "$GITHUB_SHA" >> "$GITHUB_OUTPUT"' <<<"$inputs_job" || {
  echo "push-gateway-inputs must decide with scripts/push-gateway-image-inputs-changed.sh on main pushes" >&2
  exit 1
}
# The step's exit status must be the script's, so a failing check fails the
# run instead of writing an empty decision that skips both gateway builds.
decision_run=$(grep -F 'run: scripts/push-gateway-image-inputs-changed.sh' <<<"$inputs_job" | sed 's/^ *run: //')
decision_dir=$(mktemp -d)
mkdir "$decision_dir/scripts"
printf '#!/usr/bin/env bash\nexit 1\n' >"$decision_dir/scripts/push-gateway-image-inputs-changed.sh"
chmod +x "$decision_dir/scripts/push-gateway-image-inputs-changed.sh"
if (cd "$decision_dir" && GITHUB_SHA=HEAD GITHUB_OUTPUT="$decision_dir/out" bash -e -c "$decision_run"); then
  rm -rf "$decision_dir"
  echo "push-gateway-inputs must fail when the decision script fails" >&2
  exit 1
fi
rm -rf "$decision_dir"
# The check trusts the source label of :main, so :main must move only after
# the image's provenance is attested: the first publication leaves it out, and
# a later step adds it.
grep -Fxq '            if [ "$tag" = "ghcr.io/block/buzz-push-gateway:main" ]; then main=true; continue; fi' <<<"$gateway_merge_job" || {
  echo "push gateway first publication must leave out :main" >&2
  exit 1
}
attest_line=$(grep -nFx "      - name: Attest gateway image provenance" <<<"$gateway_merge_job" | cut -d: -f1)
main_line=$(grep -nFx "      - name: Publish :main" <<<"$gateway_merge_job" | cut -d: -f1)
[[ -n "$attest_line" && -n "$main_line" && "$main_line" -gt "$attest_line" ]] &&
  grep -Fxq '        run: docker buildx imagetools create -t ghcr.io/block/buzz-push-gateway:main "ghcr.io/block/buzz-push-gateway@${GATEWAY_DIGEST}"' <<<"$gateway_merge_job" || {
  echo "push gateway :main must move only after provenance is attested" >&2
  exit 1
}

# Run the decision script against a fixture repository, with `docker` stubbed
# to report the :main image the registry would return.
inputs_changed="$repo_root/scripts/push-gateway-image-inputs-changed.sh"
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir "$fixture/bin" "$fixture/repo"
cat >"$fixture/bin/docker" <<'STUB'
#!/usr/bin/env bash
[[ "$*" == "buildx imagetools inspect ghcr.io/block/buzz-push-gateway:main --format {{json .Image}}" ]] || exit 1
[[ -n "${STUB_IMAGE_JSON:-}" ]] || exit 1
printf '%s\n' "$STUB_IMAGE_JSON"
STUB
chmod +x "$fixture/bin/docker"
(
  cd "$fixture/repo"
  export PATH="$fixture/bin:$PATH"
  git init -q
  git config user.email ci@example.org
  git config user.name ci
  git config commit.gpgsign false
  git config core.hooksPath /dev/null
  mkdir -p crates/buzz-push-gateway/src crates/buzz-relay/src
  echo 'fn main() {}' >crates/buzz-push-gateway/src/main.rs
  echo 'fn main() {}' >crates/buzz-relay/src/main.rs
  echo '[workspace]' >Cargo.toml
  echo 'channel = "1.0.0"' >rust-toolchain.toml
  git add -A && git commit -qm base
  published=$(git rev-parse HEAD)
  label() {
    jq -cn --arg rev "$1" '{config: {Labels: {"org.opencontainers.image.revision": $rev}}}'
  }
  multi_arch() {
    jq -cn --argjson image "$(label "$1")" '{"linux/amd64": $image, "linux/arm64": $image}'
  }
  expect() {
    local want=$1 main_push=$2 image=$3 why=$4 got
    got=$(MAIN_PUSH=$main_push STUB_IMAGE_JSON=$image "$inputs_changed" HEAD 2>/dev/null)
    [[ "$got" == "build=$want" ]] || {
      echo "push gateway inputs check: $why: want $want, got $got" >&2
      exit 1
    }
  }

  expect false true "$(multi_arch "$published")" "a multi-platform :main at HEAD"
  expect false true "$(label "$published")" "a single-platform :main at HEAD"
  expect true false "$(multi_arch "$published")" "an event other than a main push"
  expect true true "" "a :main lookup failure"
  echo '// relay only' >>crates/buzz-relay/src/main.rs
  git commit -qam relay
  expect false true "$(multi_arch "$published")" "a relay-only change"
  echo 'channel = "1.0.1"' >rust-toolchain.toml
  git commit -qam toolchain
  expect true true "$(multi_arch "$published")" "a toolchain-only change"
  git reset -q --hard HEAD~1
  echo '// gateway' >>crates/buzz-push-gateway/src/main.rs
  git commit -qam gateway
  echo '// relay again' >>crates/buzz-relay/src/main.rs
  git commit -qam relay-again
  expect true true "$(multi_arch "$published")" "a gateway change behind a later relay-only commit"
  expect true true "$(multi_arch 0000000000000000000000000000000000000000)" "a source missing from history"
  git checkout -q --orphan unrelated "$published"
  git commit -qm unrelated
  expect true true "$(multi_arch "$published")" "a source that is not an ancestor"
)

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
