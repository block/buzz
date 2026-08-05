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
printf '# Changelog\n' > "$tmp/CHANGELOG.md"
echo root > "$tmp/ROOT.md"
git -C "$tmp" add .
git -C "$tmp" commit -qm 'feat: root content'
prior_base=$(git -C "$tmp" rev-parse HEAD)

# The prior immutable candidate lives on side history after its squash merge.
git -C "$tmp" checkout -qb prior-candidate
echo prior > "$tmp/desktop/feature"
cat > "$tmp/.release/desktop-candidate.json" <<JSON
{"schema":1,"version":"1.0.0","base_sha":"$prior_base","previous_tag":null,"tag":"desktop-v1.0.0","commit_count":1}
JSON
git -C "$tmp" add .
git -C "$tmp" commit -qm 'chore(release): release Buzz Desktop version 1.0.0'
prior_candidate=$(git -C "$tmp" rev-parse HEAD)
git -C "$tmp" -c tag.gpgSign=false tag desktop-v1.0.0

git -C "$tmp" checkout -q -
echo before-squash > "$tmp/POLICY.md"
git -C "$tmp" add POLICY.md
git -C "$tmp" commit -qm 'chore(release): unrelated hostile subject'
unrelated_before=$(git -C "$tmp" rev-parse HEAD)
echo squash > "$tmp/PRIOR_RELEASE.md"
git -C "$tmp" add PRIOR_RELEASE.md
git -C "$tmp" commit -qm 'edited prior release subject'
prior_merge=$(git -C "$tmp" rev-parse HEAD)
echo after-squash >> "$tmp/desktop/feature"
git -C "$tmp" add desktop/feature
git -C "$tmp" commit -qm 'fix: desktop fix after prior release'
unrelated_after=$(git -C "$tmp" rev-parse HEAD)
base=$(git -C "$tmp" rev-parse HEAD)

mock_bin=$(mktemp -d)
cat > "$mock_bin/gh" <<GH
#!/usr/bin/env bash
[[ "\$1" == api && "\$2" == "repos/block/buzz/commits/$prior_candidate/pulls" ]] || exit 90
cat <<JSON
[{"merged_at":"2026-01-01T00:00:00Z","merge_commit_sha":"$prior_merge","head":{"sha":"$prior_candidate"}}]
JSON
GH
chmod +x "$mock_bin/gh"
(
  cd "$tmp"
  PATH="$mock_bin:$PATH" scripts/desktop_release.py generate 1.0.1 --base "$base" --repo block/buzz
  python3 - <<'PY'
import json
for path in ('desktop/package.json', 'desktop/src-tauri/tauri.conf.json'):
    data=json.load(open(path)); data['version']='1.0.1'; open(path,'w').write(json.dumps(data)+'\n')
open('desktop/src-tauri/Cargo.toml','w').write('[package]\nversion = "1.0.1"\n')
PY
  git add .
  git -c user.name=Wes -c user.email=wesbillman@users.noreply.github.com commit -q -s -m 'chore(release): release Buzz Desktop version 1.0.1' -m 'Co-authored-by: Test Automation <test@example.com>'
  PATH="$mock_bin:$PATH" scripts/desktop_release.py validate --version 1.0.1 --repo block/buzz
  grep -Fq "$unrelated_before" CHANGELOG.md
  grep -Fq "$unrelated_after" CHANGELOG.md
  ! grep -Fq "$prior_merge" CHANGELOG.md
  jq -e --arg base "$prior_base" --arg merge "$prior_merge" \
    '.schema == 2 and .previous_tag == "desktop-v1.0.0" and .previous_base_sha == $base and .previous_merge_sha == $merge' \
    .release/desktop-candidate.json >/dev/null

  cp .release/desktop-candidate.json metadata.json
  jq '.previous_merge_sha = "0000000000000000000000000000000000000000"' metadata.json > .release/desktop-candidate.json
  if PATH="$mock_bin:$PATH" scripts/desktop_release.py validate --version 1.0.1 --repo block/buzz >/dev/null 2>&1; then
    echo "validator accepted a forged previous release ledger" >&2; exit 1
  fi
  mv metadata.json .release/desktop-candidate.json

  # Post-merge verification may be retried after this candidate's immutable tag
  # already exists. Accept only the exact candidate SHA; an equal-version tag
  # anywhere else remains a collision.
  candidate=$(git rev-parse HEAD)
  git -c tag.gpgSign=false tag desktop-v1.0.1 "$candidate"
  PATH="$mock_bin:$PATH" scripts/desktop_release.py validate --version 1.0.1 --repo block/buzz
  git -c tag.gpgSign=false tag -f desktop-v1.0.1 "$base" >/dev/null
  if PATH="$mock_bin:$PATH" scripts/desktop_release.py validate --version 1.0.1 --repo block/buzz >/dev/null 2>&1; then
    echo "validator accepted an equal-version tag at the wrong SHA" >&2; exit 1
  fi
  git tag -d desktop-v1.0.1 >/dev/null

  # Prerelease tags are not prior-release ledgers, but the exact target tag is
  # still a collision boundary: same-SHA retry passes; wrong-SHA reuse fails.
  git -c tag.gpgSign=false tag desktop-v1.0.1-beta "$candidate"
  PATH="$mock_bin:$PATH" python3 - <<'PY'
import importlib.util
import pathlib

spec = importlib.util.spec_from_file_location("desktop_release", pathlib.Path("scripts/desktop_release.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
candidate = module.git("rev-parse", "HEAD")
module.previous_release("1.0.1-beta", "block/buzz", allow_target_sha=candidate)
PY
  git -c tag.gpgSign=false tag -f desktop-v1.0.1-beta "$base" >/dev/null
  if PATH="$mock_bin:$PATH" python3 - <<'PY'
import importlib.util
import pathlib

spec = importlib.util.spec_from_file_location("desktop_release", pathlib.Path("scripts/desktop_release.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
candidate = module.git("rev-parse", "HEAD")
module.previous_release("1.0.1-beta", "block/buzz", allow_target_sha=candidate)
PY
  then
    echo "validator accepted a prerelease target tag at the wrong SHA" >&2; exit 1
  fi
  git tag -d desktop-v1.0.1-beta >/dev/null

  # A stable tag with the same numeric tuple is a different tag and cannot
  # authorize a prerelease retry, even when it points at the candidate.
  git -c tag.gpgSign=false tag desktop-v1.0.1 "$candidate"
  if PATH="$mock_bin:$PATH" python3 - <<'PY'
import importlib.util
import pathlib

spec = importlib.util.spec_from_file_location("desktop_release", pathlib.Path("scripts/desktop_release.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
candidate = module.git("rev-parse", "HEAD")
module.previous_release("1.0.1-beta", "block/buzz", allow_target_sha=candidate)
PY
  then
    echo "validator accepted a mismatched stable tag for a prerelease target" >&2; exit 1
  fi
  git tag -d desktop-v1.0.1 >/dev/null
)

# Equal and decreasing versions are rejected before any GitHub lookup.
for invalid_version in 1.0.0 0.9.9; do
  if (cd "$tmp" && PATH="/usr/bin:/bin" scripts/desktop_release.py generate "$invalid_version" --base "$base" --repo block/buzz) >/dev/null 2>&1; then
    echo "generator accepted non-increasing version $invalid_version" >&2; exit 1
  fi
done

# A production-style schema-1 tag points at its squash commit on main. It must
# still resolve as the prior ledger during migration to head-tagged releases.
migration=$(mktemp -d)
git clone -q "$tmp" "$migration"
git -C "$migration" config user.name test
git -C "$migration" config user.email test@example.com
git -C "$migration" checkout -q "$prior_base"
GIT_EDITOR=true git -C "$migration" cherry-pick "$prior_candidate" >/dev/null
production_tag=$(git -C "$migration" rev-parse HEAD)
git -C "$migration" -c tag.gpgSign=false tag -f desktop-v1.0.0 "$production_tag" >/dev/null
echo migration >> "$migration/desktop/feature"
git -C "$migration" add desktop/feature
git -C "$migration" commit -qm 'fix: migration change'
migration_base=$(git -C "$migration" rev-parse HEAD)
cat > "$mock_bin/gh" <<GH
#!/usr/bin/env bash
[[ "\$1" == api && "\$2" == "repos/block/buzz/commits/$production_tag/pulls" ]] || exit 90
printf '%s\n' '[{"merged_at":"2026-01-01T00:00:00Z","merge_commit_sha":"$production_tag","head":{"sha":"$prior_candidate"}}]'
GH
chmod +x "$mock_bin/gh"
(cd "$migration" && PATH="$mock_bin:$PATH" scripts/desktop_release.py generate 1.0.1 --base "$migration_base" --repo block/buzz)
jq -e --arg merge "$production_tag" '.previous_tag == "desktop-v1.0.0" and .previous_merge_sha == $merge' "$migration/.release/desktop-candidate.json" >/dev/null
rm -rf "$migration"

# An initial release still accounts for the root commit without calling GitHub.
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
