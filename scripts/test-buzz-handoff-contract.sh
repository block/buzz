#!/usr/bin/env bash
# Focused, relay-free contract tests for hardened journal handoffs.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/buzz-handoff-contract.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

fail() {
  echo "test-buzz-handoff-contract: $*" >&2
  exit 1
}

CLAIMANT="2b6e9ea5607dff3a3eb0c805409ed22f240a2341628d76df6015b2bfeee0dcd2"
OPEN_ID="49a1cda764e9aa76791af9ebf12aa945180a9d6e95e3d194d949806cd44c539f"
CLAIM_ID="025f6b99b6f392ffb2031470d6ac61b0217e6f905894d4f9492a3f2900f5ae46"
RETURN_ID="c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3"
CONTEXT="5b2ab726-c2b0-4bce-bcb9-bf5f7f883f0b"

HOME_DIR="$TMP/home"
BIN_DIR="$HOME_DIR/bin"
mkdir -p "$BIN_DIR" "$TMP/path"
printf 'test-key\n' > "$HOME_DIR/node.key"

cat > "$BIN_DIR/sovereign-client" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "${1:-}" in
  pubkey)
    printf '%s\n' "${TEST_CLAIMANT:?}"
    ;;
  query)
    filter="${4:-}"
    if [ -n "${TEST_RUNNER_OPEN:-}" ]; then
      jq -cn --arg id "$TEST_OPEN_ID" --arg pk "$TEST_CLAIMANT" \
        --arg base "$TEST_BASE" --arg context "$TEST_CONTEXT" '
        [{
          id:$id, pubkey:$pk, created_at:10, kind:1,
          tags:[["t","handoff:open"],["h",$context],["p",$pk]],
          content:({
            title:"contract fixture",
            scope:"scripts only",
            base_commit:$base,
            acceptance:"tests pass",
            embodiment:{
              stdin:"closed",network:"none",trust:"fixture",tooling:"shell"
            },
            artifacts:[]
          }|tojson),
          sig:("0"*128)
        }]'
    elif [ -n "${TEST_VERIFY_RETURN:-}" ]; then
      jq -cn --arg id "$TEST_RETURN_ID" --arg pk "$TEST_CLAIMANT" \
        --arg context "$TEST_CONTEXT" --arg sha "$TEST_ARTIFACT_SHA" '
        [{
          id:$id,pubkey:$pk,created_at:30,kind:1,
          tags:[
            ["t","handoff:return"],["h",$context],["x",$sha],
            ["e",("a"*64),"","root"],["e",("b"*64),"","claim"]
          ],
          content:({status:"done",evidence:"fixture",artifacts:[$sha]}|tojson),
          sig:("0"*128)
        }]'
    elif [[ "$filter" == *'"#x"'* ]]; then
      if [ "${TEST_MANIFEST_MISSING:-0}" = "1" ]; then
        printf '200 []\n'
      else
        printf '200 [{"id":"manifest"}]\n'
      fi
    else
      printf '200 []\n'
    fi
    ;;
  fetch)
    printf 'artifact-bytes' > "${5:?fetch output path missing}"
    printf 'fetched\n'
    ;;
  upload)
    file="${4:?upload file missing}"
    sha="$(shasum -a 256 "$file" | awk '{print $1}')"
    size="$(wc -c < "$file" | tr -d ' ')"
    printf '%s\n' "${3:?upload relay missing}" >> "${TEST_UPLOAD_LOG:?}"
    printf '200 {"sha256":"%s","size":%s}\n' "$sha" "$size"
    ;;
  post)
    cp "${4:?event file missing}" "${TEST_CAPTURE:?}"
    printf '200 {"accepted":true,"message":"stored"}\n'
    printf 'event_id=%064d\n' 0
    ;;
  *)
    echo "unexpected sovereign-client command: ${1:-}" >&2
    exit 1
    ;;
esac
EOF
chmod +x "$BIN_DIR/sovereign-client"

cat > "$BIN_DIR/buzz-relay-push" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf 'synced\n' > "${TEST_SYNC_MARK:?}"
EOF
chmod +x "$BIN_DIR/buzz-relay-push"

cat > "$BIN_DIR/buzz-handoff-state" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
cat >/dev/null
state_json() {
  if [ "${TEST_STATE:-CLAIMED}" = "RETURNED" ]; then
    jq -cn --arg open "$TEST_OPEN_ID" --arg context "$TEST_CONTEXT" \
    --arg claimant "$TEST_CLAIMANT" --arg claim "$TEST_CLAIM_ID" \
    --arg returned "$TEST_RETURN_ID" --arg owner "${TEST_OPEN_OWNER:-$(printf 'b%.0s' {1..64})}" '
    {
      open_id:$open,title:"fixture",context:$context,
      opener_owner_pubkey:$owner,allowed_claimants:[$claimant],
      state:"RETURNED",claim_id:$claim,claimant_pubkey:$claimant,
      created_at:10,claim_created_at:20,return_created_at:30,
      return_id:$returned
    }'
  else
    jq -cn --arg open "$TEST_OPEN_ID" --arg context "$TEST_CONTEXT" \
    --arg claimant "$TEST_CLAIMANT" --arg claim "$TEST_CLAIM_ID" '
    {
      open_id:$open,title:"fixture",context:$context,
      opener_owner_pubkey:("b"*64),allowed_claimants:[$claimant],
      state:"CLAIMED",claim_id:$claim,claimant_pubkey:$claimant,
      created_at:10,claim_created_at:20,return_created_at:null,
      return_id:null
    }'
  fi
}
if [ "${1:-}" = "--open" ]; then
  state_json
else
  state_json | jq -c '[.]'
fi
EOF
chmod +x "$BIN_DIR/buzz-handoff-state"

cat > "$TMP/path/curl" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "$TMP/path/curl"

cat > "$TMP/fake-buzz-ctx" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "$TMP/fake-buzz-ctx"

export TEST_CLAIMANT="$CLAIMANT"
export TEST_OPEN_ID="$OPEN_ID"
export TEST_CLAIM_ID="$CLAIM_ID"
export TEST_RETURN_ID="$RETURN_ID"
export TEST_CONTEXT="$CONTEXT"
export BUZZ_CTX_HOME="$HOME_DIR"
export BUZZ_HANDOFF_STATE="$BIN_DIR/buzz-handoff-state"
export BUZZ_CTX_LOCAL="http://local.example"
export BUZZ_CTX_CLOUD="https://cloud.example"
export BUZZ_CTX_AGENT="fixture"
export PATH="$TMP/path:$PATH"

bash -n "$ROOT/scripts/buzz-ctx" "$ROOT/scripts/buzz-runner" "$ROOT/scripts/buzz-steward"

ARTIFACT_SHA="$(printf 'artifact-bytes' | shasum -a 256 | awk '{print $1}')"
export TEST_ARTIFACT_SHA="$ARTIFACT_SHA"
printf 'artifact-bytes' > "$TMP/artifact"
export TEST_CAPTURE="$TMP/manifest-event.json"
export TEST_UPLOAD_LOG="$TMP/uploads.log"
export TEST_SYNC_MARK="$TMP/synced"
"$ROOT/scripts/buzz-ctx" announce "$TMP/artifact" "contract artifact" \
  > "$TMP/announce.out"
[ "$(wc -l < "$TEST_UPLOAD_LOG" | tr -d ' ')" -eq 2 ] \
  || fail "announce did not upload bytes to both sovereign and rendezvous stores"
grep -q '^http://local.example$' "$TEST_UPLOAD_LOG" \
  || fail "announce skipped sovereign artifact custody"
grep -q '^https://cloud.example$' "$TEST_UPLOAD_LOG" \
  || fail "announce skipped rendezvous artifact custody"
[ -s "$TEST_SYNC_MARK" ] \
  || fail "announce did not synchronize the manifest before custody verification"
grep -q 'rendezvous custody: verified' "$TMP/announce.out" \
  || fail "announce did not report verified rendezvous custody"

RETURN_SPEC="$TMP/return.json"
jq -n --arg sha "$ARTIFACT_SHA" \
  '{status:"done",evidence:"contract proof",artifacts:[$sha]}' > "$RETURN_SPEC"

export TEST_CAPTURE="$TMP/return-event.json"
"$ROOT/scripts/buzz-ctx" handoff return "$OPEN_ID" "$RETURN_SPEC" >/dev/null
jq -e --arg open "$OPEN_ID" --arg claim "$CLAIM_ID" --arg sha "$ARTIFACT_SHA" '
  (.tags | any(. == ["e",$open,"","root"]))
  and (.tags | any(. == ["e",$claim,"","claim"]))
  and (.tags | any(. == ["x",$sha]))
  and ((.content | fromjson).claim_id == $claim)
' "$TEST_CAPTURE" >/dev/null \
  || fail "return did not bind root, claim, and rendezvous-custodied artifact"

export TEST_VERIFY_RETURN=1
export TEST_STATE=RETURNED
"$ROOT/scripts/buzz-ctx" handoff verify-artifacts "$RETURN_ID" \
  > "$TMP/verify-artifacts.out"
grep -q "verified artifact: $ARTIFACT_SHA" "$TMP/verify-artifacts.out" \
  || fail "independent return verification did not fetch and hash its artifact"
unset TEST_VERIFY_RETURN TEST_STATE

export TEST_MANIFEST_MISSING=1
export TEST_CAPTURE="$TMP/missing-event.json"
if "$ROOT/scripts/buzz-ctx" handoff return "$OPEN_ID" "$RETURN_SPEC" \
    >"$TMP/missing.out" 2>&1; then
  fail "return advertised an artifact with no rendezvous manifest"
fi
[ ! -e "$TEST_CAPTURE" ] \
  || fail "artifact-gate failure still posted a return event"
unset TEST_MANIFEST_MISSING

export TEST_STATE=RETURNED
export TEST_CAPTURE="$TMP/close-event.json"
if "$ROOT/scripts/buzz-ctx" handoff close "$OPEN_ID" "$RETURN_ID" \
    >"$TMP/close.out" 2>&1; then
  fail "a signer outside the opener owner identity closed the handoff"
fi
[ ! -e "$TEST_CAPTURE" ] \
  || fail "unauthorized close reached event publication"

export TEST_OPEN_OWNER="$CLAIMANT"
export TEST_VERIFY_RETURN=1
"$ROOT/scripts/buzz-ctx" handoff close "$OPEN_ID" "$RETURN_ID" \
  > "$TMP/authorized-close.out"
jq -e --arg open "$OPEN_ID" --arg returned "$RETURN_ID" '
  (.tags | any(. == ["e",$open,"","root"]))
  and (.tags | any(. == ["e",$returned,"","return"]))
  and ((.content | fromjson).return_id == $returned)
' "$TEST_CAPTURE" >/dev/null \
  || fail "authorized close did not bind the independently verified return"
grep -q 'return custody verified' "$TMP/authorized-close.out" \
  || fail "close did not execute independent rendezvous custody verification"
unset TEST_OPEN_OWNER TEST_VERIFY_RETURN
unset TEST_STATE

REPO="$TMP/repo"
mkdir -p "$REPO"
git -C "$REPO" init -q
git -C "$REPO" config user.name "Contract Test"
git -C "$REPO" config user.email "contract@example.invalid"
printf 'fixture\n' > "$REPO/README"
git -C "$REPO" add README
git -C "$REPO" commit -q -m "fixture"
TEST_BASE="$(git -C "$REPO" rev-parse HEAD)"
export TEST_BASE TEST_RUNNER_OPEN=1
export BUZZ_RUNNER_REPOSITORY="$REPO"
export BUZZ_RUNNER_TRUSTED_REF="HEAD"
export BUZZ_RUNNER_BUZZ_CTX="$TMP/fake-buzz-ctx"

"$ROOT/scripts/buzz-runner" "$OPEN_ID" --dry-run --isolation host \
  > "$TMP/runner-dry.out"
grep -q 'lifecycle:  CLAIMED' "$TMP/runner-dry.out" \
  || fail "runner dry-run did not use signer-derived lifecycle state"

TEST_BASE='HEAD~1'
export TEST_BASE
if "$ROOT/scripts/buzz-runner" "$OPEN_ID" --dry-run --isolation host \
    >"$TMP/runner-symbolic.out" 2>&1; then
  fail "runner accepted a symbolic base revision"
fi
grep -q 'base_commit must be a canonical lowercase 40-character object id' \
  "$TMP/runner-symbolic.out" \
  || fail "runner symbolic-base refusal was not explicit"
TEST_BASE="$(git -C "$REPO" rev-parse HEAD)"
export TEST_BASE

if "$ROOT/scripts/buzz-runner" "$OPEN_ID" --isolation host \
    >"$TMP/runner-host.out" 2>&1; then
  fail "runner permitted unattended host execution"
fi
grep -q 'unattended host execution is disabled' "$TMP/runner-host.out" \
  || fail "runner host refusal was not explicit"
grep -q 'env -i ' "$ROOT/scripts/buzz-runner" \
  || fail "runner execution environment is not an explicit allowlist"
if grep -q 'env -u ' "$ROOT/scripts/buzz-runner"; then
  fail "runner retained an environment denylist"
fi
[ ! -e "$REPO/.claude/worktrees/runner-${OPEN_ID:0:12}" ] \
  || fail "pre-claim host refusal left its execution worktree behind"
if git -C "$REPO" worktree list --porcelain \
    | grep -q "runner-${OPEN_ID:0:12}"; then
  fail "pre-claim host refusal left a registered worktree behind"
fi

mkdir -p "$TMP/installed"
cp "$ROOT/scripts/buzz-runner" "$TMP/installed/buzz-runner"
unset BUZZ_RUNNER_REPOSITORY
if "$TMP/installed/buzz-runner" "$OPEN_ID" --dry-run \
    >"$TMP/installed.out" 2>&1; then
  fail "installed runner guessed a repository"
fi
grep -q 'installed copies require BUZZ_RUNNER_REPOSITORY' "$TMP/installed.out" \
  || fail "installed runner did not require an explicit trusted repository"

echo "buzz handoff contract tests passed"
