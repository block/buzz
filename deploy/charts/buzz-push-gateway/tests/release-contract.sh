#!/usr/bin/env bash
set -euo pipefail
env -u GEM_HOME -u GEM_PATH -u RUBYLIB -u RUBYOPT ruby -ryaml <<'RUBY'
auto_text = File.read('.github/workflows/auto-tag-on-release-pr-merge.yml')
publish_text = File.read('.github/workflows/push-gateway-helm-chart.yml')
deployment_text = File.read('docs/push-gateway-deployment.md')
chart = YAML.load_file('deploy/charts/buzz-push-gateway/Chart.yaml')
# Parse first, then pin the tag producer and consumer strings whose agreement
# makes this a reachable lane rather than an orphan publisher.
YAML.load(auto_text)
YAML.load(publish_text)
version = chart.fetch('version').to_s
raise "gateway chart version is not semver: #{version}" unless version.match?(/\A\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?\z/)
workspace_package = File.read('Cargo.toml').match(/\[workspace\.package\](.*?)(?=\n\[|\z)/m)
raise "workspace package metadata is missing" unless workspace_package
binary_version = workspace_package[1].match(/^version\s*=\s*"([^"]+)"/)&.[](1)
raise "workspace package version is missing" unless binary_version
unless chart.fetch('appVersion').to_s == binary_version
  raise "gateway chart appVersion does not match packaged binary #{binary_version}"
end
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
[
  'inspect and fetch the published chart version',
  'helm show chart oci://ghcr.io/block/buzz/charts/buzz-push-gateway --version X.Y.Z',
  'helm pull oci://ghcr.io/block/buzz/charts/buzz-push-gateway --version X.Y.Z',
].each do |needle|
  raise "missing gateway chart retrieval guidance: #{needle}" unless deployment_text.include?(needle)
end
if deployment_text.include?('verify the immutable chart artifact')
  raise 'gateway chart retrieval guidance overstates authenticity verification'
end
RUBY
