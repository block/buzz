#!/usr/bin/env bash
set -euo pipefail
chart=deploy/charts/buzz-push-gateway
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
helm template push "$chart" >"$out/default.yaml"
helm template push "$chart" \
  --set-string 'podLabels.example\.com/runtime=gateway' \
  --set serviceAccountName=mesh-gateway \
  --set terminationGracePeriodSeconds=90 >"$out/runtime.yaml"
helm template push "$chart" --set networkPolicy.enabled=false \
  --set networkPolicy.externalPolicyName=platform-runtime >"$out/external.yaml"
env -u GEM_HOME -u GEM_PATH -u RUBYLIB -u RUBYOPT ruby -ryaml - "$out" <<'RUBY'
base, runtime, external = %w[default runtime external].map { |name| YAML.load_stream(File.read("#{ARGV[0]}/#{name}.yaml")).compact }
pod = runtime.find { |r| r['kind'] == 'Deployment' }.dig('spec', 'template')
raise 'runtime label missing' unless pod['metadata']['labels'].delete('example.com/runtime') == 'gateway'
raise 'service account missing' unless pod['spec'].delete('serviceAccountName') == 'mesh-gateway'
raise 'grace period missing' unless pod['spec']['terminationGracePeriodSeconds'] == 90
pod['spec']['terminationGracePeriodSeconds'] = 60
raise 'runtime options changed unrelated resources or selectors' unless runtime == base
expected = base.reject { |r| r['kind'] == 'NetworkPolicy' && r['metadata']['name'] == 'push-buzz-push-gateway' }
raise 'external policy mode changed migration isolation or other resources' unless external == expected
RUBY
reject() {
  if helm template push "$chart" "$@" >"$out/invalid.log" 2>&1; then
    echo "expected invalid runtime options to fail: $*" >&2
    exit 1
  fi
  if ! grep -q 'schema' "$out/invalid.log"; then
    cat "$out/invalid.log" >&2
    exit 1
  fi
}
reject --set networkPolicy.enabled=false
reject --set networkPolicy.enabled=false --set-string 'networkPolicy.externalPolicyName= '
reject --set networkPolicy.externalPolicyName=conflicting-policy
reject --set terminationGracePeriodSeconds=59
reject --set podLabels.invalid=true
for label in name instance component; do
  reject --set-string "podLabels.app\\.kubernetes\\.io/$label=override"
done

# Kubernetes NetworkPolicy names are DNS subdomains, not single DNS labels.
long_name=$(printf '%0253d' 0)
for name in runtime.platform.example "$long_name"; do
  helm template push "$chart" --set networkPolicy.enabled=false \
    --set-string "networkPolicy.externalPolicyName=$name" >/dev/null
done
for name in "$long_name"x .runtime runtime. runtime..example Runtime runtime.-example; do
  reject --set networkPolicy.enabled=false --set-string "networkPolicy.externalPolicyName=$name"
done

# Parent policy owns scrape access in external mode. Internal policy retains its guard.
helm template push "$chart" --set networkPolicy.enabled=false \
  --set networkPolicy.externalPolicyName=platform-runtime \
  --set podMonitor.enabled=true >"$out/external-monitor.yaml"
env -u GEM_HOME -u GEM_PATH -u RUBYLIB -u RUBYOPT ruby -ryaml - "$out/external-monitor.yaml" <<'RUBY'
resources = YAML.load_stream(File.read(ARGV[0])).compact
raise 'PodMonitor missing in external mode' unless resources.any? { |r| r['kind'] == 'PodMonitor' }
policies = resources.select { |r| r['kind'] == 'NetworkPolicy' }
raise 'external mode must retain only migration policy' unless policies.map { |r| r['metadata']['name'] } == ['push-buzz-push-gateway-migration']
RUBY
reject --set podMonitor.enabled=true
reject --set podMonitor.enabled=true --set networkPolicy.monitoring.enabled=true

# External ownership replaces runtime egress only; migration still needs DB/DNS.
helm template push "$chart" --set networkPolicy.enabled=false \
  --set networkPolicy.externalPolicyName=platform-runtime \
  --set-json 'networkPolicy.apnsEgressCidrs=[]' >/dev/null
reject --set-json 'networkPolicy.apnsEgressCidrs=[]'
reject --set networkPolicy.enabled=false --set networkPolicy.externalPolicyName=platform-runtime \
  --set-json 'networkPolicy.postgresEgressCidrs=[]'
reject --set networkPolicy.enabled=false --set networkPolicy.externalPolicyName=platform-runtime \
  --set networkPolicy.dns=null
reject --set terminationGracePeriodSeconds=null
