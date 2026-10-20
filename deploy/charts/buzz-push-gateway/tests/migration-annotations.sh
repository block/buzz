#!/usr/bin/env bash
set -euo pipefail
chart=deploy/charts/buzz-push-gateway
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
helm template push "$chart" >"$out/default.yaml"
helm template push "$chart" \
  --set-string 'deploymentAnnotations.secret\.reloader\.stakater\.com/reload=buzz-push-gateway' \
  --set-string 'migration.podAnnotations.sidecar\.istio\.io/inject=false' \
  --set-string 'podAnnotations.example\.com/runtime=enabled' >"$out/annotated.yaml"
env -u GEM_HOME -u GEM_PATH -u RUBYLIB -u RUBYOPT ruby -ryaml - "$out" <<'RUBY'
base, changed = %w[default annotated].map { |name| YAML.load_stream(File.read("#{ARGV[0]}/#{name}.yaml")).compact }
job = changed.find { |r| r['kind'] == 'Job' }
raise 'migration annotation missing or not a string' unless job.dig('spec', 'template', 'metadata').delete('annotations') == {'sidecar.istio.io/inject' => 'false'}
runtime = changed.find { |r| r['kind'] == 'Deployment' }
raise 'Deployment annotation missing' unless runtime['metadata'].delete('annotations') == {'secret.reloader.stakater.com/reload' => 'buzz-push-gateway'}
raise 'runtime annotations changed' unless runtime.dig('spec', 'template', 'metadata').delete('annotations') == {'example.com/runtime' => 'enabled'}
raise 'annotations changed other resources or hook metadata' unless base == changed
RUBY
for value in true 7; do
  if helm template push "$chart" --set "migration.podAnnotations.invalid=$value" >"$out/invalid.log" 2>&1; then
    echo 'expected non-string migration annotation to fail schema validation' >&2
    exit 1
  fi
  if ! grep -Eq 'Expected: string|want string' "$out/invalid.log"; then
    cat "$out/invalid.log" >&2
    exit 1
  fi
done

for key in release-namespace external-network-policy; do
  if helm template push "$chart" --set-string "deploymentAnnotations.buzz\.block\.xyz/$key=override" >"$out/invalid.log" 2>&1; then
    echo 'expected reserved Deployment annotation to fail schema validation' >&2
    exit 1
  fi
  grep -q 'deploymentAnnotations' "$out/invalid.log"
done
if helm template push "$chart" --set deploymentAnnotations.invalid=true >"$out/invalid.log" 2>&1; then
  echo 'expected non-string Deployment annotation to fail schema validation' >&2
  exit 1
fi
grep -q 'deploymentAnnotations' "$out/invalid.log"
helm template push "$chart" --namespace tenant-runtime \
  --set networkPolicy.enabled=false --set networkPolicy.externalPolicyName=platform-runtime \
  --set-string 'deploymentAnnotations.secret\.reloader\.stakater\.com/reload=buzz-push-gateway' >"$out/external.yaml"
env -u GEM_HOME -u GEM_PATH -u RUBYLIB -u RUBYOPT ruby -ryaml - "$out/external.yaml" <<'RUBY'
resources = YAML.load_stream(File.read(ARGV[0])).compact
deployment = resources.find { |r| r['kind'] == 'Deployment' }
raise 'ownership annotations not preserved' unless deployment['metadata']['annotations'] == {
  'secret.reloader.stakater.com/reload' => 'buzz-push-gateway',
  'buzz.block.xyz/release-namespace' => 'tenant-runtime',
  'buzz.block.xyz/external-network-policy' => 'platform-runtime'
}
RUBY
