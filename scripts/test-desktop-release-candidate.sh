#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cp "$repo_root/scripts/desktop_release.py" "$tmp/desktop_release.py"

git -C "$tmp" init -q
git -C "$tmp" config user.name test
git -C "$tmp" config user.email test@example.com
mkdir -p "$tmp/scripts" "$tmp/desktop/src-tauri" "$tmp/crates/buzz-core" "$tmp/.release"
mv "$tmp/desktop_release.py" "$tmp/scripts/desktop_release.py"
printf '{"version":"1.0.0"}\n' > "$tmp/desktop/package.json"
printf '{"version":"1.0.0"}\n' > "$tmp/desktop/src-tauri/tauri.conf.json"
printf '[package]\nversion = "1.0.0"\n' > "$tmp/desktop/src-tauri/Cargo.toml"
echo '# Changelog' > "$tmp/CHANGELOG.md"
echo first > "$tmp/desktop/feature"
git -C "$tmp" add .
git -C "$tmp" commit -qm 'feat: first desktop change'
git -C "$tmp" -c tag.gpgSign=false tag v1.0.0
echo second >> "$tmp/desktop/feature"
git -C "$tmp" commit -qam 'fix: desktop fix'
echo policy > "$tmp/POLICY.md"
git -C "$tmp" add POLICY.md
git -C "$tmp" commit -qm 'docs: repository policy'
base=$(git -C "$tmp" rev-parse HEAD)
(
  cd "$tmp"
  scripts/desktop_release.py generate 1.0.1 --base "$base" --repo block/buzz
  python3 - <<'PY'
import json
for path in ('desktop/package.json', 'desktop/src-tauri/tauri.conf.json'):
    data=json.load(open(path)); data['version']='1.0.1'; open(path,'w').write(json.dumps(data)+'\n')
p='desktop/src-tauri/Cargo.toml'; open(p,'w').write('[package]\nversion = "1.0.1"\n')
PY
  rm -f msg
  git add .
  cat >msg <<'EOF'
chore(release): release Buzz Desktop version 1.0.1

Co-authored-by: Test Automation <test@example.com>
EOF
  git -c user.name=Wes -c user.email=wesbillman@users.noreply.github.com commit -q -s -F msg
  rm msg
  scripts/desktop_release.py validate --version 1.0.1 --repo block/buzz
  grep -Fq '### Other repository changes' CHANGELOG.md
  grep -Fq "$(git rev-parse HEAD~1)" CHANGELOG.md
  grep -Fq "$(git rev-parse HEAD~2)" CHANGELOG.md

  # Metadata cannot lie about the prior release boundary.
  cp .release/desktop-candidate.json metadata.json
  python3 - <<'PY'
import json
p='.release/desktop-candidate.json'; d=json.load(open(p)); d['previous_tag']=None; open(p,'w').write(json.dumps(d)+'\n')
PY
  if scripts/desktop_release.py validate --version 1.0.1 --repo block/buzz >/dev/null 2>&1; then
    echo "validator accepted a forged previous release tag" >&2
    exit 1
  fi
  mv metadata.json .release/desktop-candidate.json
)

# An initial release must account for the root commit, not silently omit it.
initial=$(mktemp -d)
cp "$repo_root/scripts/desktop_release.py" "$initial/desktop_release.py"
git -C "$initial" init -q
git -C "$initial" config user.name test
git -C "$initial" config user.email test@example.com
mkdir -p "$initial/scripts" "$initial/desktop/src-tauri"
mv "$initial/desktop_release.py" "$initial/scripts/desktop_release.py"
printf '{"version":"0.1.0"}\n' > "$initial/desktop/package.json"
printf '{"version":"0.1.0"}\n' > "$initial/desktop/src-tauri/tauri.conf.json"
printf '[package]\nversion = "0.1.0"\n' > "$initial/desktop/src-tauri/Cargo.toml"
printf '# Changelog\n' > "$initial/CHANGELOG.md"
echo root > "$initial/ROOT.md"
git -C "$initial" add .
git -C "$initial" commit -qm 'feat: root release content'
root_sha=$(git -C "$initial" rev-parse HEAD)
(cd "$initial" && scripts/desktop_release.py generate 0.1.0 --base "$root_sha" --repo block/buzz)
grep -Fq "$root_sha" "$initial/CHANGELOG.md"
rm -rf "$initial"

echo "desktop release candidate contract passed"
