#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT

mkdir -p "$test_root/bin" "$test_root/checkout/desktop/src-tauri/icons"
touch "$test_root/checkout/desktop/src-tauri/icons/icon.icns"

cat >"$test_root/bin/git" <<EOF
#!/usr/bin/env bash
case "\$*" in
  "rev-parse --show-toplevel") printf '%s\n' '$test_root/checkout' ;;
  "rev-parse --is-inside-work-tree") exit 0 ;;
  "rev-parse --git-dir") printf '%s\n' '.git/worktrees/test' ;;
  "rev-parse --git-common-dir") printf '%s\n' '.git' ;;
  "rev-parse --abbrev-ref HEAD") printf '%s\n' 'HEAD' ;;
  *) exit 1 ;;
esac
EOF
chmod +x "$test_root/bin/git"

cat >"$test_root/bin/swift" <<'EOF'
#!/usr/bin/env bash
mkdir -p "$(dirname "$3")"
touch "$3"
EOF
chmod +x "$test_root/bin/swift"

result=$(
  PATH="$test_root/bin:$PATH"
  BUZZ_DEV_LABEL='voice "route" \ fix'
  BUZZ_SHARE_IDENTITY=0
  source "$repo_root/scripts/instance-env.sh"
  printf '%s\n%s\n%s\n' \
    "$BUZZ_WORKTREE_LABEL" "$BUZZ_INSTANCE_SLUG" "$BUZZ_TAURI_CONFIG"
)

[[ "$result" == *$'voice "route" \ fix\nvoice-route-fix\n'* ]]
config=$(printf '%s\n' "$result" | tail -1)
[[ "$(node -e 'const c=JSON.parse(process.argv[1]); process.stdout.write(c.productName)' "$config")" == 'Buzz Dev (voice "route" \ fix)' ]]

echo "instance-env tests passed"
