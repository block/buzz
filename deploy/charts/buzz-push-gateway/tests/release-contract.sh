#!/usr/bin/env bash
set -euo pipefail
env -u GEM_HOME -u GEM_PATH -u RUBYLIB -u RUBYOPT ruby -ryaml <<'RUBY'
auto_text = File.read('.github/workflows/auto-tag-on-release-pr-merge.yml')
publish_text = File.read('.github/workflows/push-gateway-helm-chart.yml')
# Parse first, then pin the tag producer and consumer strings whose agreement
# makes this a reachable lane rather than an orphan publisher.
YAML.load(auto_text)
YAML.load(publish_text)
[
  'push-chart-release/*)',
  'VERSION="${BRANCH#push-chart-release/}"',
  'TAG_PREFIX="push-chart-v"',
  '- name: Create and push tag',
  'TAG: ${{ steps.release.outputs.tag }}',
  'refs/tags/$TAG',
  '-f sha="$TARGET_SHA"',
].each do |needle|
  raise "missing auto-tag gateway chart contract: #{needle}" unless auto_text.include?(needle)
end
[
  'tags: ["push-chart-v[0-9]*"]',
  'version="${INPUT_VERSION:-${REF_NAME#push-chart-v}}"',
  'refs/tags/push-chart-v${version}^{commit}',
  'deploy/charts/buzz-push-gateway',
].each do |needle|
  raise "missing gateway chart publisher contract: #{needle}" unless publish_text.include?(needle)
end
RUBY
