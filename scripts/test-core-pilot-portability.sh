#!/usr/bin/env bash
# Contract tests for exporting the Core pilot from one clean checkout and
# importing it into another without transferring credentials or runtime data.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail() {
  printf 'FAIL: %s\n' "$1" >&2
  exit 1
}

source_repo="$tmp/source"
mkdir -p "$source_repo"
git -C "$source_repo" init -q
git -C "$source_repo" config user.email core-pilot-test@example.invalid
git -C "$source_repo" config user.name 'Core Pilot Test'
printf 'public base\n' > "$source_repo/base.txt"
git -C "$source_repo" add base.txt
git -C "$source_repo" commit -q -m base
base_commit="$(git -C "$source_repo" rev-parse HEAD)"

mkdir -p "$source_repo/scripts" "$source_repo/config/core-pilot"
cp "$repo_root/scripts/core-pilot-lib.sh" "$source_repo/scripts/"
cp "$repo_root/config/core-pilot/core-research-partner.md" "$source_repo/config/core-pilot/"
[[ -f "$repo_root/scripts/core-pilot-export.sh" ]] \
  || fail 'export workflow is missing'
cp "$repo_root/scripts/core-pilot-export.sh" "$source_repo/scripts/"
[[ -f "$repo_root/scripts/core-pilot-import.sh" ]] \
  || fail 'import workflow is missing'
cp "$repo_root/scripts/core-pilot-import.sh" "$source_repo/scripts/"
git -C "$source_repo" add scripts config
git -C "$source_repo" commit -q -m pilot

secrets_dir="$tmp/source-config"
state_dir="$tmp/source-state"
mkdir -p "$secrets_dir" "$state_dir"
cat > "$secrets_dir/agent.env" <<'EOF'
OPENAI_COMPAT_API_KEY=SENTINEL_API_CREDENTIAL
CORE_RELAY_PUBLIC_KEY=79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798
CORE_RELAY_PRIVATE_KEY=0000000000000000000000000000000000000000000000000000000000000001
CORE_BANKER_PUBLIC_KEY=c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5
CORE_BANKER_PRIVATE_KEY=0000000000000000000000000000000000000000000000000000000000000002
CORE_AGENT_PUBLIC_KEY=f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9
CORE_AGENT_PRIVATE_KEY=0000000000000000000000000000000000000000000000000000000000000003
CORE_NON_OWNER_PUBLIC_KEY=e493dbf1c10d80f3581e4904930b1404cc6c13900ee0758474fa94abe8c4cd13
CORE_NON_OWNER_PRIVATE_KEY=0000000000000000000000000000000000000000000000000000000000000004
EOF
cat > "$state_dir/channels.env" <<'EOF'
CORE_RESEARCH_CHANNEL_ID=33333333-3333-4333-8333-333333333333
CORE_SECOND_CHANNEL_ID=44444444-4444-4444-8444-444444444444
EOF
chmod 600 "$secrets_dir/agent.env" "$state_dir/channels.env"

fake_bin="$tmp/fake-bin"
mkdir -p "$fake_bin"
export GPG_CAPTURE="$tmp/gpg.calls"
export GPG_FD_MARKER="$tmp/gpg-fd.marker"
cat > "$fake_bin/gpg" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
environment="$(env)"
if [[ "$environment" == *SENTINEL_API_CREDENTIAL* \
   || "$environment" == *SENTINEL_TRANSFER_PASSPHRASE* \
   || "$environment" == *0000000000000000000000000000000000000000000000000000000000000001* ]]; then
  printf 'gpg mock received secret-bearing environment\n' >&2
  exit 97
fi
printf 'gpg' >> "$GPG_CAPTURE"
printf ' <%q>' "$@" >> "$GPG_CAPTURE"
printf '\n' >> "$GPG_CAPTURE"
output=
passphrase_fd=
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output) output="$2"; shift 2 ;;
    --passphrase-fd) passphrase_fd="$2"; shift 2 ;;
    --decrypt) input="$2"; shift 2 ;;
    *) shift ;;
  esac
done
if [[ -n "$passphrase_fd" ]]; then
  printf '%s' "$passphrase_fd" > "$GPG_FD_MARKER"
fi
if [[ -n "${input:-}" ]]; then
  cp "$input" "$output"
else
  cat > "$output"
fi
EOF
chmod +x "$fake_bin/gpg"
cat > "$fake_bin/chmod" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${CHMOD_FAIL_PRIVATE_TEMP:-}" == 1 && "$*" == *core-pilot-import.* ]]; then
  exit 98
fi
if [[ -f "${GPG_FD_MARKER:-}" ]]; then
  descriptor="$(<"$GPG_FD_MARKER")"
  if [[ "$descriptor" =~ ^[0-9]+$ && -e "/proc/$$/fd/$descriptor" ]]; then
    printf 'post-GPG command inherited the passphrase descriptor\n' >&2
    exit 97
  fi
fi
exec /usr/bin/chmod "$@"
EOF
chmod +x "$fake_bin/chmod"
cat > "$fake_bin/openssl" <<'EOF'
#!/usr/bin/env bash
# Fast deterministic boundary fake for repeated parser/import cases. A separate
# assertion below exercises the real OpenSSL SEC1 derivation once.
set -euo pipefail
der_hex="$(/usr/bin/xxd -p -c 1000)"
private_key="${der_hex:14:64}"
case "$private_key" in
  0000000000000000000000000000000000000000000000000000000000000001)
    public_key=79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798 ;;
  0000000000000000000000000000000000000000000000000000000000000002)
    public_key=c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5 ;;
  0000000000000000000000000000000000000000000000000000000000000003)
    public_key=f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9 ;;
  0000000000000000000000000000000000000000000000000000000000000004)
    public_key=e493dbf1c10d80f3581e4904930b1404cc6c13900ee0758474fa94abe8c4cd13 ;;
  *) exit 1 ;;
esac
printf '04%s%064d' "$public_key" 0 | /usr/bin/xxd -r -p
EOF
chmod +x "$fake_bin/openssl"

in_repo_state="$source_repo/untracked-state"
mkdir -p "$in_repo_state"
cp "$secrets_dir/agent.env" "$source_repo/untracked-agent.env"
cp "$state_dir/channels.env" "$in_repo_state/channels.env"
chmod 600 "$source_repo/untracked-agent.env" "$in_repo_state/channels.env"
set +e
inside_input_output="$({
  cd "$source_repo"
  PATH="$fake_bin:$PATH" ./scripts/core-pilot-export.sh \
    --output "$tmp/inside-input-transfer" \
    --base "$base_commit" \
    --secrets "$source_repo/untracked-agent.env" \
    --state-dir "$in_repo_state"
} 2>&1)"
inside_input_status=$?
set -e
[[ $inside_input_status -ne 0 && ! -e "$tmp/inside-input-transfer" \
   && "$inside_input_output" != *0000000000000000000000000000000000000000000000000000000000000001* ]] \
  || fail 'export accepted private source state stored inside the repository'
rm -f "$in_repo_state/channels.env" "$source_repo/untracked-agent.env"
rmdir "$in_repo_state"
printf 'ok: export rejects private input paths inside the repository\n'

printf 'dirty tracked content\n' >> "$source_repo/base.txt"
set +e
dirty_output="$({
  cd "$source_repo"
  PATH="$fake_bin:$PATH" ./scripts/core-pilot-export.sh \
    --output "$tmp/dirty-transfer" \
    --base "$base_commit" \
    --secrets "$secrets_dir/agent.env" \
    --state-dir "$state_dir"
} 2>&1)"
dirty_status=$?
set -e
git -C "$source_repo" restore base.txt
[[ $dirty_status -ne 0 && ! -e "$tmp/dirty-transfer" \
   && "$dirty_output" != *0000000000000000000000000000000000000000000000000000000000000001* ]] \
  || fail 'export accepted dirty tracked source content'

set +e
inside_output="$({
  cd "$source_repo"
  PATH="$fake_bin:$PATH" ./scripts/core-pilot-export.sh \
    --output "$source_repo/untracked-transfer" \
    --base "$base_commit" \
    --secrets "$secrets_dir/agent.env" \
    --state-dir "$state_dir"
} 2>&1)"
inside_status=$?
set -e
[[ $inside_status -ne 0 && ! -e "$source_repo/untracked-transfer" \
   && "$inside_output" != *0000000000000000000000000000000000000000000000000000000000000001* ]] \
  || fail 'export accepted an output destination inside the repository'

existing_output="$tmp/existing-transfer"
mkdir -m 700 "$existing_output"
printf 'preserve\n' > "$existing_output/marker"
set +e
existing_output_message="$({
  cd "$source_repo"
  PATH="$fake_bin:$PATH" ./scripts/core-pilot-export.sh \
    --output "$existing_output" \
    --base "$base_commit" \
    --secrets "$secrets_dir/agent.env" \
    --state-dir "$state_dir"
} 2>&1)"
existing_output_status=$?
set -e
[[ $existing_output_status -ne 0 && "$(cat "$existing_output/marker")" == preserve ]] \
  || fail 'export overwrote or accepted an existing output destination'

set +e
missing_state_message="$({
  cd "$source_repo"
  PATH="$fake_bin:$PATH" ./scripts/core-pilot-export.sh \
    --output "$tmp/missing-state-transfer" \
    --base "$base_commit" \
    --secrets "$tmp/missing-agent.env" \
    --state-dir "$tmp/missing-state"
} 2>&1)"
missing_state_status=$?
set -e
[[ $missing_state_status -ne 0 && ! -e "$tmp/missing-state-transfer" \
   && "$missing_state_message" == *'pilot identity file'* ]] \
  || fail 'export did not fail clearly when no portable identity state exists'
printf 'ok: export rejects dirty code and unsafe or existing output destinations\n'

artifact_dir="$tmp/transfer"
output="$({
  cd "$source_repo"
  PATH="$fake_bin:$PATH" ./scripts/core-pilot-export.sh \
    --output "$artifact_dir" \
    --base "$base_commit" \
    --secrets "$secrets_dir/agent.env" \
    --state-dir "$state_dir"
} 2>&1)" || fail "valid export failed: $output"

[[ -f "$artifact_dir/core-pilot.bundle" \
   && -f "$artifact_dir/core-pilot-state.gpg" \
   && -f "$artifact_dir/SHA256SUMS" ]] \
  || fail 'valid export did not create both transfer artifacts and their manifest'
[[ "$output" != *SENTINEL_API_CREDENTIAL* \
   && "$output" != *0000000000000000000000000000000000000000000000000000000000000001* ]] \
  || fail 'valid export printed a secret'
expected_source_commit="$(git -C "$source_repo" rev-parse HEAD)"
expected_bundle_sha256="$(sha256sum "$artifact_dir/core-pilot.bundle" | awk '{print $1}')"
[[ "$output" == *"Expected source commit (record separately): $expected_source_commit"* \
   && "$output" == *"Expected bundle SHA-256 (record separately): $expected_bundle_sha256"* ]] \
  || fail 'export did not print out-of-band provenance values'
grep -q '^CORE_PILOT_TRANSFER_SCHEMA=1$' "$artifact_dir/core-pilot-state.gpg" \
  || fail 'encrypted state input is not schema-versioned'
if grep -q 'OPENAI_COMPAT_API_KEY\|SENTINEL_API_CREDENTIAL' "$artifact_dir/core-pilot-state.gpg"; then
  fail 'exported private state included the OpenAI credential'
fi
actual_transfer_keys="$(cut -d= -f1 "$artifact_dir/core-pilot-state.gpg")"
expected_transfer_keys="$(cat <<'EOF'
CORE_PILOT_TRANSFER_SCHEMA
CORE_PILOT_SOURCE_COMMIT
CORE_PILOT_BUNDLE_BASE
CORE_PILOT_PROMPT_SHA256
CORE_RELAY_PUBLIC_KEY
CORE_RELAY_PRIVATE_KEY
CORE_BANKER_PUBLIC_KEY
CORE_BANKER_PRIVATE_KEY
CORE_AGENT_PUBLIC_KEY
CORE_AGENT_PRIVATE_KEY
CORE_NON_OWNER_PUBLIC_KEY
CORE_NON_OWNER_PRIVATE_KEY
CORE_RESEARCH_CHANNEL_ID
CORE_SECOND_CHANNEL_ID
EOF
)"
[[ "$actual_transfer_keys" == "$expected_transfer_keys" ]] \
  || fail 'exported private state contains fields outside the exact transfer schema'
[[ "$(find "$artifact_dir" -mindepth 1 -maxdepth 1 -type f | wc -l)" -eq 3 \
   && "$(stat -c '%a:%u' "$artifact_dir")" == "700:$UID" \
   && "$(stat -c '%a:%u' "$artifact_dir/core-pilot.bundle")" == "600:$UID" \
   && "$(stat -c '%a:%u' "$artifact_dir/core-pilot-state.gpg")" == "600:$UID" \
   && "$(stat -c '%a:%u' "$artifact_dir/SHA256SUMS")" == "600:$UID" ]] \
  || fail 'export created unexpected files or unsafe artifact permissions'
(
  cd "$artifact_dir"
  sha256sum --check --strict SHA256SUMS >/dev/null
) || fail 'transfer manifest does not verify both artifacts'

printf 'ok: valid export creates a bundle and credential-free encrypted state\n'

worktree_repo="$tmp/source-worktree"
git -C "$source_repo" branch worktree-export
git -C "$source_repo" worktree add -q "$worktree_repo" worktree-export
worktree_transfer="$tmp/worktree-transfer"
worktree_output="$({
  cd "$worktree_repo"
  PATH="$fake_bin:$PATH" ./scripts/core-pilot-export.sh \
    --output "$worktree_transfer" \
    --base "$base_commit" \
    --secrets "$secrets_dir/agent.env" \
    --state-dir "$state_dir"
} 2>&1)" || fail "linked-worktree export failed: $worktree_output"
git -C "$source_repo" bundle verify "$worktree_transfer/core-pilot.bundle" >/dev/null 2>&1 \
  || fail 'linked-worktree export did not produce a verifiable bundle'
printf 'ok: export works from a linked Git worktree\n'

modified_prompt_repo="$tmp/modified-prompt-worktree"
git -C "$source_repo" branch modified-prompt-export
git -C "$source_repo" worktree add -q "$modified_prompt_repo" modified-prompt-export
printf '\nunreviewed committed instruction\n' \
  >> "$modified_prompt_repo/config/core-pilot/core-research-partner.md"
git -C "$modified_prompt_repo" add config/core-pilot/core-research-partner.md
git -C "$modified_prompt_repo" commit -q -m 'unreviewed prompt'
set +e
modified_prompt_output="$({
  cd "$modified_prompt_repo"
  PATH="$fake_bin:$PATH" ./scripts/core-pilot-export.sh \
    --output "$tmp/modified-prompt-transfer" \
    --base "$base_commit" \
    --secrets "$secrets_dir/agent.env" \
    --state-dir "$state_dir"
} 2>&1)"
modified_prompt_status=$?
set -e
[[ $modified_prompt_status -ne 0 && ! -e "$tmp/modified-prompt-transfer" ]] \
  || fail 'export accepted a clean commit containing an unreviewed Core prompt'
printf 'ok: export pins the independently reviewed prompt digest\n'

destination_repo="$tmp/destination-repo"
git -C "$tmp" init -q destination-repo
git -C "$destination_repo" fetch -q "$source_repo" "$base_commit:refs/heads/public-base"
git -C "$destination_repo" checkout -q public-base
git -C "$destination_repo" fetch -q "$artifact_dir/core-pilot.bundle" HEAD:refs/heads/core-pilot
git -C "$destination_repo" checkout -q core-pilot
[[ "$(git -C "$destination_repo" rev-parse HEAD)" == "$(git -C "$source_repo" rev-parse HEAD)" ]] \
  || fail 'incremental bundle did not reconstruct the source commit from its prerequisite'

destination_secrets="$tmp/destination-config/core-buzz/agent.env"
destination_state="$tmp/destination-state/core-buzz"
passphrase_file="$tmp/passphrase"
printf 'SENTINEL_TRANSFER_PASSPHRASE\n' > "$passphrase_file"
chmod 600 "$passphrase_file"
import_output="$({
  cd "$destination_repo"
  PATH="$fake_bin:$PATH" ./scripts/core-pilot-import.sh \
    --source "$artifact_dir" \
    --secrets "$destination_secrets" \
    --state-dir "$destination_state" \
    --passphrase-fd 3 3<"$passphrase_file"
} 2>&1)" || fail "valid import failed: $import_output"

[[ "$import_output" != *SENTINEL_TRANSFER_PASSPHRASE* \
   && "$import_output" != *0000000000000000000000000000000000000000000000000000000000000001* ]] \
  || fail 'valid import printed a secret or passphrase'
[[ "$(stat -c '%a:%u' "$destination_secrets")" == "600:$UID" \
   && "$(stat -c '%a:%u' "$destination_state/channels.env")" == "600:$UID" ]] \
  || fail 'imported state is not current-user owned and mode 0600'
grep -q '^OPENAI_COMPAT_API_KEY=$' "$destination_secrets" \
  || fail 'imported identity file did not leave the OpenAI credential empty'
grep -q '^CORE_RESEARCH_CHANNEL_ID=33333333-3333-4333-8333-333333333333$' \
  "$destination_state/channels.env" \
  || fail 'import did not preserve the research channel UUID'
[[ ! -e "$tmp/destination-config/core-buzz/pilot.env" \
   && ! -e "$destination_state/relay.log" \
   && ! -e "$destination_state/relay.pid" ]] \
  || fail 'import recreated excluded generated or runtime state'

printf 'ok: incremental bundle fetch and private-state import reconstruct the portable pilot\n'

identity_hash_before="$(sha256sum "$destination_secrets")"
channel_hash_before="$(sha256sum "$destination_state/channels.env")"
repeat_output="$({
  cd "$destination_repo"
  PATH="$fake_bin:$PATH" ./scripts/core-pilot-import.sh \
    --source "$artifact_dir" \
    --secrets "$destination_secrets" \
    --state-dir "$destination_state"
} 2>&1)" || fail "identical re-import failed: $repeat_output"
[[ "$(sha256sum "$destination_secrets")" == "$identity_hash_before" \
   && "$(sha256sum "$destination_state/channels.env")" == "$channel_hash_before" ]] \
  || fail 'identical re-import changed portable state'
printf 'ok: identical private-state import is idempotent\n'

cp "$destination_secrets" "$tmp/expected-agent.env"
sed -i \
  's/^CORE_NON_OWNER_PRIVATE_KEY=.*/CORE_NON_OWNER_PRIVATE_KEY=0000000000000000000000000000000000000000000000000000000000000005/' \
  "$destination_secrets"
different_hash="$(sha256sum "$destination_secrets")"
set +e
different_output="$({
  cd "$destination_repo"
  PATH="$fake_bin:$PATH" ./scripts/core-pilot-import.sh \
    --source "$artifact_dir" \
    --secrets "$destination_secrets" \
    --state-dir "$destination_state"
} 2>&1)"
different_status=$?
set -e
[[ $different_status -ne 0 && "$(sha256sum "$destination_secrets")" == "$different_hash" \
   && "$different_output" != *0000000000000000000000000000000000000000000000000000000000000001* ]] \
  || fail 'import overwrote different existing identity state or leaked it'
cp "$tmp/expected-agent.env" "$destination_secrets"
chmod 600 "$destination_secrets"

chmod 644 "$destination_state/channels.env"
set +e
permissive_destination_output="$({
  cd "$destination_repo"
  PATH="$fake_bin:$PATH" ./scripts/core-pilot-import.sh \
    --source "$artifact_dir" \
    --secrets "$destination_secrets" \
    --state-dir "$destination_state"
} 2>&1)"
permissive_destination_status=$?
set -e
chmod 600 "$destination_state/channels.env"
[[ $permissive_destination_status -ne 0 ]] \
  || fail 'import accepted permissive existing destination state'
printf 'ok: import refuses different or permissive existing state\n'

uppercase_transfer="$tmp/uppercase-transfer"
cp -a "$artifact_dir" "$uppercase_transfer"
sed -i \
  -e 's/^CORE_RELAY_PUBLIC_KEY=.*/CORE_RELAY_PUBLIC_KEY=79BE667EF9DCBBAC55A06295CE870B07029BFCDB2DCE28D959F2815B16F81798/' \
  -e 's/^CORE_RESEARCH_CHANNEL_ID=.*/CORE_RESEARCH_CHANNEL_ID=AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA/' \
  "$uppercase_transfer/core-pilot-state.gpg"
(
  cd "$uppercase_transfer"
  sha256sum -- core-pilot.bundle core-pilot-state.gpg > .SHA256SUMS.new
  chmod 600 .SHA256SUMS.new
  mv .SHA256SUMS.new SHA256SUMS
)
uppercase_secrets="$tmp/uppercase-config/core-buzz/agent.env"
uppercase_state="$tmp/uppercase-state/core-buzz"
uppercase_output="$({
  cd "$destination_repo"
  PATH="$fake_bin:$PATH" ./scripts/core-pilot-import.sh \
    --source "$uppercase_transfer" \
    --secrets "$uppercase_secrets" \
    --state-dir "$uppercase_state"
} 2>&1)" || fail "uppercase canonicalization import failed: $uppercase_output"
grep -q '^CORE_RELAY_PUBLIC_KEY=79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798$' \
  "$uppercase_secrets" \
  && grep -q '^CORE_RESEARCH_CHANNEL_ID=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa$' \
    "$uppercase_state/channels.env" \
  || fail 'imported identity or channel state was not canonicalized to lowercase'
printf 'ok: private-state import canonicalizes hexadecimal identities and UUIDs\n'

refresh_manifest() {
  local directory="$1"
  (
    cd "$directory"
    sha256sum -- core-pilot.bundle core-pilot-state.gpg > .SHA256SUMS.test
    chmod 600 .SHA256SUMS.test
    mv .SHA256SUMS.test SHA256SUMS
  )
}

variant_number=0
make_variant() {
  variant_number=$((variant_number + 1))
  VARIANT_DIR="$tmp/variant-$variant_number"
  cp -a "$artifact_dir" "$VARIANT_DIR"
}

rejection_number=0
import_tmp_root="$tmp/import-tmp"
mkdir -m 700 "$import_tmp_root"
assert_import_rejected() {
  local name="$1" transfer_source="$2" output status target_secrets target_state
  rejection_number=$((rejection_number + 1))
  target_secrets="$tmp/rejected-config-$rejection_number/core-buzz/agent.env"
  target_state="$tmp/rejected-state-$rejection_number/core-buzz"
  set +e
  output="$({
    cd "$destination_repo"
    TMPDIR="$import_tmp_root" PATH="$fake_bin:$PATH" ./scripts/core-pilot-import.sh \
      --source "$transfer_source" \
      --secrets "$target_secrets" \
      --state-dir "$target_state"
  } 2>&1)"
  status=$?
  set -e
  [[ $status -ne 0 ]] || fail "$name was accepted"
  [[ ! -e "$target_secrets" && ! -e "$target_state/channels.env" ]] \
    || fail "$name wrote destination state before failing"
  [[ "$output" != *SENTINEL_API_CREDENTIAL* \
     && "$output" != *SENTINEL_TRANSFER_PASSPHRASE* \
     && "$output" != *0000000000000000000000000000000000000000000000000000000000000001* ]] \
    || fail "$name printed a secret"
  [[ -z "$(find "$import_tmp_root" -mindepth 1 -print -quit)" ]] \
    || fail "$name left decrypted temporary state behind"
}

make_variant
printf 'CORE_PILOT_UNEXPECTED_FIELD=value\n' >> "$VARIANT_DIR/core-pilot-state.gpg"
refresh_manifest "$VARIANT_DIR"
assert_import_rejected 'unknown transfer field' "$VARIANT_DIR"

make_variant
printf 'CORE_AGENT_PUBLIC_KEY=f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9\n' \
  >> "$VARIANT_DIR/core-pilot-state.gpg"
refresh_manifest "$VARIANT_DIR"
assert_import_rejected 'duplicate transfer field' "$VARIANT_DIR"

make_variant
printf 'MALFORMED RECORD\n' >> "$VARIANT_DIR/core-pilot-state.gpg"
refresh_manifest "$VARIANT_DIR"
assert_import_rejected 'malformed transfer record' "$VARIANT_DIR"

make_variant
printf '\0' >> "$VARIANT_DIR/core-pilot-state.gpg"
refresh_manifest "$VARIANT_DIR"
assert_import_rejected 'binary transfer record' "$VARIANT_DIR"

make_variant
sed -i 's/^CORE_AGENT_PRIVATE_KEY=.*/CORE_AGENT_PRIVATE_KEY=not-a-key/' \
  "$VARIANT_DIR/core-pilot-state.gpg"
refresh_manifest "$VARIANT_DIR"
assert_import_rejected 'malformed portable identity' "$VARIANT_DIR"

make_variant
sed -i \
  's/^CORE_AGENT_PRIVATE_KEY=.*/CORE_AGENT_PRIVATE_KEY=0000000000000000000000000000000000000000000000000000000000000000/' \
  "$VARIANT_DIR/core-pilot-state.gpg"
refresh_manifest "$VARIANT_DIR"
assert_import_rejected 'invalid zero private identity scalar' "$VARIANT_DIR"

make_variant
sed -i \
  's/^CORE_NON_OWNER_PUBLIC_KEY=.*/CORE_NON_OWNER_PUBLIC_KEY=c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5/' \
  "$VARIANT_DIR/core-pilot-state.gpg"
refresh_manifest "$VARIANT_DIR"
assert_import_rejected 'colliding portable public identities' "$VARIANT_DIR"

make_variant
sed -i \
  's/^CORE_NON_OWNER_PRIVATE_KEY=.*/CORE_NON_OWNER_PRIVATE_KEY=0000000000000000000000000000000000000000000000000000000000000002/' \
  "$VARIANT_DIR/core-pilot-state.gpg"
refresh_manifest "$VARIANT_DIR"
assert_import_rejected 'colliding portable private identities' "$VARIANT_DIR"

make_variant
sed -i \
  -e 's/^CORE_RELAY_PUBLIC_KEY=.*/CORE_RELAY_PUBLIC_KEY=SWAPPED_PUBLIC_KEY/' \
  -e 's/^CORE_BANKER_PUBLIC_KEY=.*/CORE_BANKER_PUBLIC_KEY=79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798/' \
  -e 's/^CORE_RELAY_PUBLIC_KEY=SWAPPED_PUBLIC_KEY$/CORE_RELAY_PUBLIC_KEY=c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5/' \
  "$VARIANT_DIR/core-pilot-state.gpg"
refresh_manifest "$VARIANT_DIR"
assert_import_rejected 'mismatched portable public/private identity pair' "$VARIANT_DIR"

make_variant
sed -i 's/^CORE_SECOND_CHANNEL_ID=.*/CORE_SECOND_CHANNEL_ID=not-a-uuid/' \
  "$VARIANT_DIR/core-pilot-state.gpg"
refresh_manifest "$VARIANT_DIR"
assert_import_rejected 'malformed portable channel UUID' "$VARIANT_DIR"

make_variant
sed -i "s/^CORE_PILOT_SOURCE_COMMIT=.*/CORE_PILOT_SOURCE_COMMIT=$base_commit/" \
  "$VARIANT_DIR/core-pilot-state.gpg"
refresh_manifest "$VARIANT_DIR"
assert_import_rejected 'mismatched portable source commit' "$VARIANT_DIR"

make_variant
sed -i 's/^CORE_PILOT_PROMPT_SHA256=.*/CORE_PILOT_PROMPT_SHA256=ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff/' \
  "$VARIANT_DIR/core-pilot-state.gpg"
refresh_manifest "$VARIANT_DIR"
assert_import_rejected 'mismatched reviewed prompt hash' "$VARIANT_DIR"

make_variant
printf 'copy corruption\n' >> "$VARIANT_DIR/core-pilot-state.gpg"
assert_import_rejected 'artifact checksum mismatch' "$VARIANT_DIR"

git -C "$destination_repo" config user.email core-pilot-test@example.invalid
git -C "$destination_repo" config user.name 'Core Pilot Test'
git -C "$destination_repo" switch -q -c wrong-destination-commit
git -C "$destination_repo" commit -q --allow-empty -m mismatch
assert_import_rejected 'destination checkout commit mismatch' "$artifact_dir"
git -C "$destination_repo" switch -q core-pilot

symlink_source="$tmp/transfer-source-link"
ln -s "$artifact_dir" "$symlink_source"
assert_import_rejected 'symlinked transfer source directory' "$symlink_source"

make_variant
rm "$VARIANT_DIR/core-pilot-state.gpg"
ln -s "$artifact_dir/core-pilot-state.gpg" "$VARIANT_DIR/core-pilot-state.gpg"
assert_import_rejected 'symlinked encrypted artifact' "$VARIANT_DIR"

make_variant
chmod 644 "$VARIANT_DIR/core-pilot-state.gpg"
assert_import_rejected 'permissive encrypted artifact' "$VARIANT_DIR"

unsafe_temp_real="$tmp/unsafe-temp-real"
unsafe_temp_link="$tmp/unsafe-temp-link"
mkdir -m 700 "$unsafe_temp_real"
ln -s "$unsafe_temp_real" "$unsafe_temp_link"
unsafe_temp_secrets="$tmp/unsafe-temp-config/core-buzz/agent.env"
unsafe_temp_state="$tmp/unsafe-temp-state/core-buzz"
set +e
unsafe_temp_output="$({
  cd "$destination_repo"
  TMPDIR="$unsafe_temp_link" PATH="$fake_bin:$PATH" ./scripts/core-pilot-import.sh \
    --source "$artifact_dir" \
    --secrets "$unsafe_temp_secrets" \
    --state-dir "$unsafe_temp_state"
} 2>&1)"
unsafe_temp_status=$?
set -e
[[ $unsafe_temp_status -ne 0 && ! -e "$unsafe_temp_secrets" \
   && ! -e "$unsafe_temp_state/channels.env" ]] \
  || fail 'import accepted a symlinked temporary workspace parent'

chmod_failure_tmp="$tmp/chmod-failure-tmp"
mkdir -m 700 "$chmod_failure_tmp"
set +e
chmod_failure_output="$({
  cd "$destination_repo"
  CHMOD_FAIL_PRIVATE_TEMP=1 TMPDIR="$chmod_failure_tmp" PATH="$fake_bin:$PATH" \
    ./scripts/core-pilot-import.sh \
      --source "$artifact_dir" \
      --secrets "$tmp/chmod-failure-config/core-buzz/agent.env" \
      --state-dir "$tmp/chmod-failure-state/core-buzz"
} 2>&1)"
chmod_failure_status=$?
set -e
[[ $chmod_failure_status -ne 0 \
   && -z "$(find "$chmod_failure_tmp" -mindepth 1 -print -quit)" ]] \
  || fail 'import left a private temporary directory after an early chmod failure'

printf 'ok: import rejects malformed, mismatched, corrupted, and unsafe transfers\n'

symlink_config="$tmp/symlink-config"
symlink_state="$tmp/symlink-state"
victim_file="$tmp/do-not-overwrite"
mkdir -m 700 "$symlink_config"
printf 'DO NOT OVERWRITE\n' > "$victim_file"
ln -s "$victim_file" "$symlink_config/.agent.env.import"
set +e
symlink_temp_output="$({
  cd "$destination_repo"
  PATH="$fake_bin:$PATH" ./scripts/core-pilot-import.sh \
    --source "$artifact_dir" \
    --secrets "$symlink_config/agent.env" \
    --state-dir "$symlink_state"
} 2>&1)"
symlink_temp_status=$?
set -e
[[ $symlink_temp_status -ne 0 && "$(cat "$victim_file")" == 'DO NOT OVERWRITE' \
   && ! -e "$symlink_config/agent.env" ]] \
  || fail 'import followed or replaced an unsafe pre-existing staging symlink'
printf 'ok: import rejects unsafe destination staging paths without overwriting them\n'

colliding_destination="$tmp/colliding-destination"
set +e
colliding_destination_output="$({
  cd "$destination_repo"
  PATH="$fake_bin:$PATH" ./scripts/core-pilot-import.sh \
    --source "$artifact_dir" \
    --secrets "$colliding_destination/channels.env" \
    --state-dir "$colliding_destination"
} 2>&1)"
colliding_destination_status=$?
set -e
[[ $colliding_destination_status -ne 0 \
   && ! -e "$colliding_destination/channels.env" ]] \
  || fail 'import accepted colliding identity and channel destination paths'
printf 'ok: import rejects colliding identity and channel destination paths\n'

permissive_channels="$tmp/permissive-channels.env"
cp "$state_dir/channels.env" "$permissive_channels"
chmod 644 "$permissive_channels"
if (
  PILOT_CHANNELS_FILE="$permissive_channels"
  source "$repo_root/scripts/core-pilot-lib.sh"
  pilot_load_channels >/dev/null 2>&1
); then
  fail 'shared channel loading accepted permissive generated state'
fi
printf 'ok: shared channel loading rejects permissive generated state\n'

uppercase_channels="$tmp/uppercase-channels.env"
cat > "$uppercase_channels" <<'EOF'
CORE_RESEARCH_CHANNEL_ID=AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA
CORE_SECOND_CHANNEL_ID=BBBBBBBB-BBBB-4BBB-8BBB-BBBBBBBBBBBB
EOF
chmod 600 "$uppercase_channels"
if ! (
  PILOT_CHANNELS_FILE="$uppercase_channels"
  source "$repo_root/scripts/core-pilot-lib.sh"
  pilot_load_channels >/dev/null
  [[ "${PILOT_CHANNELS[CORE_RESEARCH_CHANNEL_ID]}" == aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa \
     && "${PILOT_CHANNELS[CORE_SECOND_CHANNEL_ID]}" == bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb ]]
); then
  fail 'shared channel loading did not canonicalize uppercase UUIDs'
fi
printf 'ok: shared channel loading canonicalizes UUIDs\n'

[[ "$(cat "$GPG_CAPTURE")" == *'<--symmetric>'* \
   && "$(cat "$GPG_CAPTURE")" == *'<--cipher-algo> <AES256>'* \
   && "$(cat "$GPG_CAPTURE")" == *'<--decrypt>'* \
   && "$(cat "$GPG_CAPTURE")" == *'--passphrase-fd'* \
   && "$(cat "$GPG_FD_MARKER")" == 3 \
   && "$(cat "$GPG_CAPTURE")" != *SENTINEL_API_CREDENTIAL* \
   && "$(cat "$GPG_CAPTURE")" != *SENTINEL_TRANSFER_PASSPHRASE* \
   && "$(cat "$GPG_CAPTURE")" != *0000000000000000000000000000000000000000000000000000000000000001* ]] \
  || fail 'GPG was not invoked through a secret-free argv/environment boundary'
printf 'ok: GPG invocation keeps secrets out of argv/environment and closes the passphrase descriptor\n'

real_openssl="$(command -v openssl)" || fail 'real OpenSSL is unavailable for identity-pair validation'
real_public_der="$(
  printf '%s' '302e02010104200000000000000000000000000000000000000000000000000000000000000001a00706052b8104000a' \
    | /usr/bin/xxd -r -p \
    | "$real_openssl" ec -inform DER -pubout -outform DER -conv_form uncompressed 2>/dev/null \
    | /usr/bin/xxd -p -c 1000
)" || fail 'real OpenSSL could not derive a secp256k1 public identity'
[[ "$real_public_der" =~ 04([0-9a-f]{64})[0-9a-f]{64}$ \
   && "${BASH_REMATCH[1]}" == 79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798 ]] \
  || fail 'real OpenSSL derived the wrong x-only public identity'
printf 'ok: real OpenSSL validates private-to-public identity derivation\n'

real_gpg="$(command -v gpg)" || fail 'real GPG is unavailable for the portability contract'
real_gpg_home="$tmp/real-gpg-home"
mkdir -m 700 "$real_gpg_home"
printf 'schema-versioned private state test\n' > "$tmp/real-gpg-plain"
printf 'correct nonsecret test passphrase\n' > "$tmp/real-gpg-passphrase"
printf 'wrong nonsecret test passphrase\n' > "$tmp/real-gpg-wrong-passphrase"
chmod 600 "$tmp/real-gpg-plain" "$tmp/real-gpg-passphrase" "$tmp/real-gpg-wrong-passphrase"
GNUPGHOME="$real_gpg_home" "$real_gpg" --no-options --quiet --cipher-algo AES256 \
  --s2k-count 65536 --batch --pinentry-mode loopback --passphrase-fd 3 --symmetric \
  --output "$tmp/real-gpg-state.gpg" 3<"$tmp/real-gpg-passphrase" <"$tmp/real-gpg-plain"
[[ -s "$tmp/real-gpg-state.gpg" ]] \
  && ! cmp -s "$tmp/real-gpg-plain" "$tmp/real-gpg-state.gpg" \
  || fail 'real GPG did not produce ciphertext'
GNUPGHOME="$real_gpg_home" "$real_gpg" --no-options --quiet --batch \
  --pinentry-mode loopback --passphrase-fd 3 --output "$tmp/real-gpg-roundtrip" \
  --decrypt "$tmp/real-gpg-state.gpg" 3<"$tmp/real-gpg-passphrase"
cmp -s "$tmp/real-gpg-plain" "$tmp/real-gpg-roundtrip" \
  || fail 'real GPG could not decrypt the symmetric state artifact'
set +e
wrong_pass_output="$(GNUPGHOME="$real_gpg_home" "$real_gpg" --no-options --quiet --batch \
  --pinentry-mode loopback --passphrase-fd 3 --output "$tmp/real-gpg-wrong-output" \
  --decrypt "$tmp/real-gpg-state.gpg" 3<"$tmp/real-gpg-wrong-passphrase" 2>&1)"
wrong_pass_status=$?
set -e
[[ $wrong_pass_status -ne 0 \
   && "$wrong_pass_output" != *'wrong nonsecret test passphrase'* \
   && "$wrong_pass_output" != *'correct nonsecret test passphrase'* ]] \
  || fail 'real GPG accepted or printed the wrong passphrase'
GNUPGHOME="$real_gpg_home" gpgconf --kill gpg-agent >/dev/null 2>&1 || true
printf 'ok: real GPG encrypts state and rejects a wrong passphrase\n'
