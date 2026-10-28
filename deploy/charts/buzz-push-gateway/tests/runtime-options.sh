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
external.find { |r| r['kind'] == 'Deployment' }['metadata'].delete('annotations')
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

# Kubernetes DNS-subdomain service accounts and qualified label keys.
for name in '' runtime.platform.example "$long_name"; do
  helm template push "$chart" --set-string "serviceAccountName=$name" >/dev/null
done
for name in INVALID_NAME ' ' "$long_name"x .runtime runtime. runtime..example; do
  reject --set-string "serviceAccountName=$name"
done
long_label=$(printf '%063d' 0)
for key in 'example.com/Runtime_v1' "$long_label" "$long_name/$long_label"; do
  helm template push "$chart" --set-json "podLabels={\"$key\":\"valid_value\"}" >/dev/null
done
for key in 'bad key' '/name' 'prefix/' 'UPPER.example/name' 'a/b/c' "$long_label"x "$long_name"x/name; do
  reject --set-json "podLabels={\"$key\":\"value\"}"
done
reject --set-json 'podLabels={"valid":"bad value"}'

# Combined parent render gate: mutations must fail without emitting manifests.
env -u GEM_HOME -u GEM_PATH -u RUBYLIB -u RUBYOPT ruby -ryaml - "$out" <<'RUBY'
resources = YAML.load_stream(File.read("#{ARGV[0]}/external.yaml")).compact
pod = resources.find { |r| r['kind'] == 'Deployment' }
policy = {'apiVersion' => 'networking.k8s.io/v1', 'kind' => 'NetworkPolicy',
          'metadata' => {'name' => 'platform-runtime'},
          'spec' => {'podSelector' => {'matchLabels' => pod.dig('spec', 'selector', 'matchLabels').dup},
                     'policyTypes' => %w[Ingress Egress], 'ingress' => [], 'egress' => []}}
File.write("#{ARGV[0]}/combined.yaml", (resources + [policy]).map(&:to_yaml).join)
list = {'apiVersion' => 'v1', 'kind' => 'List', 'items' => [policy]}
File.write("#{ARGV[0]}/list.yaml", (resources + [list]).map(&:to_yaml).join)
File.write("#{ARGV[0]}/duplicate-list.yaml", (resources + [policy, list]).map(&:to_yaml).join)
typed_list = {'apiVersion' => 'networking.k8s.io/v1', 'kind' => 'NetworkPolicyList', 'items' => [policy]}
File.write("#{ARGV[0]}/typed-list.yaml", (resources + [typed_list]).map(&:to_yaml).join)
File.write("#{ARGV[0]}/duplicate-typed-list.yaml", (resources + [policy, typed_list]).map(&:to_yaml).join)
expression_policy = Marshal.load(Marshal.dump(policy))
identity = expression_policy['spec']['podSelector'].delete('matchLabels')
expression_policy['spec']['podSelector']['matchExpressions'] = identity.map { |k, v| {'key' => k, 'operator' => 'In', 'values' => [v]} }
File.write("#{ARGV[0]}/expression-identity.yaml", (resources + [expression_policy]).map(&:to_yaml).join)
%w[helm.sh/hook argocd.argoproj.io/hook].each_with_index do |key, i|
  wrapped = Marshal.load(Marshal.dump(typed_list))
  wrapped['metadata'] = {'annotations' => {key => i == 0 ? 'pre-upgrade' : 'PreSync'}}
  File.write("#{ARGV[0]}/wrapped-hook-#{i}.yaml", (resources + [wrapped]).map(&:to_yaml).join)
end
defaulted = Marshal.load(Marshal.dump(policy))
defaulted['spec'].delete('policyTypes')
defaulted['spec']['egress'] = [{'ports' => [{'protocol' => 'TCP', 'port' => 443}]}]
File.write("#{ARGV[0]}/defaulted-types.yaml", (resources + [defaulted]).map(&:to_yaml).join)
variants = {
  'invalid-key' => ->(p) { p['spec']['podSelector']['matchExpressions'] = [{'key' => 'bad key', 'operator' => 'DoesNotExist'}] },
  'invalid-value' => ->(p) { p['spec']['podSelector']['matchExpressions'] = [{'key' => 'absent', 'operator' => 'NotIn', 'values' => ['bad value']}] },
  'numeric-value' => ->(p) { p['spec']['podSelector']['matchExpressions'] = [{'key' => 'absent', 'operator' => 'NotIn', 'values' => [123]}] },
  'unknown-operator' => ->(p) { p['spec']['podSelector']['matchExpressions'] = [{'key' => 'absent', 'operator' => 'Unknown'}] },
  'empty-notin' => ->(p) { p['spec']['podSelector']['matchExpressions'] = [{'key' => 'absent', 'operator' => 'NotIn', 'values' => []}] },
  'valued-exists' => ->(p) { p['spec']['podSelector']['matchExpressions'] = [{'key' => 'app.kubernetes.io/name', 'operator' => 'Exists', 'values' => ['buzz-push-gateway']}] },
  'argo-skip' => ->(p) { p['metadata']['annotations'] = {'argocd.argoproj.io/hook' => 'Skip'} },
  'argo-presync' => ->(p) { p['metadata']['annotations'] = {'argocd.argoproj.io/hook' => 'PreSync', 'argocd.argoproj.io/hook-delete-policy' => 'HookSucceeded'} },
  'hook-policy' => ->(p) { p['metadata']['annotations'] = {'helm.sh/hook' => 'pre-install', 'helm.sh/hook-delete-policy' => 'hook-succeeded'} },
  'misspelled' => ->(p) { p['metadata']['name'] = 'typo' },
  'wrong-selector' => ->(p) { p['spec']['podSelector']['matchLabels']['app.kubernetes.io/instance'] = 'other' },
  'empty-selector' => ->(p) { p['spec']['podSelector'] = {} },
  'wrong-namespace' => ->(p) { p['metadata']['namespace'] = 'other' },
  'ingress-only' => ->(p) { p['spec']['policyTypes'] = ['Ingress'] },
  'wrong-expression' => ->(p) { p['spec']['podSelector']['matchExpressions'] = [{'key' => 'absent', 'operator' => 'Exists'}] }
}
variants.each do |name, mutate|
  changed = Marshal.load(Marshal.dump(policy))
  mutate.call(changed)
  File.write("#{ARGV[0]}/#{name}.yaml", (resources + [changed]).map(&:to_yaml).join)
end
RUBY
gate() { env -u GEM_HOME -u GEM_PATH -u RUBYLIB -u RUBYOPT ruby "$chart/tests/check-external-policy.rb"; }
gate <"$out/default.yaml" >"$out/gated.yaml"
cmp "$out/default.yaml" "$out/gated.yaml"
gate <"$out/combined.yaml" >"$out/gated.yaml"
cmp "$out/combined.yaml" "$out/gated.yaml"
gate <"$out/list.yaml" >"$out/gated.yaml"
cmp "$out/list.yaml" "$out/gated.yaml"
for valid in defaulted-types typed-list expression-identity; do
  gate <"$out/$valid.yaml" >"$out/gated.yaml"
  cmp "$out/$valid.yaml" "$out/gated.yaml"
done
for mutation in wrapped-hook-0 wrapped-hook-1 duplicate-typed-list argo-skip argo-presync invalid-key invalid-value numeric-value unknown-operator empty-notin valued-exists duplicate-list external hook-policy misspelled wrong-selector empty-selector wrong-namespace ingress-only wrong-expression; do
  if gate <"$out/$mutation.yaml" >"$out/gated.yaml" 2>"$out/gate-error"; then
    echo "expected combined-render gate to reject $mutation" >&2
    exit 1
  fi
  test ! -s "$out/gated.yaml"
  grep -q 'external policy' "$out/gate-error"
done

# Helm assigns omitted namespaces to the actual release namespace.
helm template push "$chart" --namespace tenant-runtime --set networkPolicy.enabled=false \
  --set networkPolicy.externalPolicyName=platform-runtime >"$out/tenant.yaml"
env -u GEM_HOME -u GEM_PATH -u RUBYLIB -u RUBYOPT ruby -ryaml - "$out" <<'RUBY'
resources = YAML.load_stream(File.read("#{ARGV[0]}/tenant.yaml")).compact
policy = YAML.load_stream(File.read("#{ARGV[0]}/combined.yaml")).compact.last
{'omitted' => nil, 'empty' => '', 'explicit' => 'tenant-runtime', 'default' => 'default', 'wrong' => 'other'}.each do |variant, namespace|
  candidate = Marshal.load(Marshal.dump(policy))
  candidate['metadata']['namespace'] = namespace if namespace
  File.write("#{ARGV[0]}/tenant-#{variant}.yaml", (resources + [candidate]).map(&:to_yaml).join)
end
# Missing release metadata must fail closed rather than assume default.
resources.find { |r| r['kind'] == 'Deployment' }['metadata']['annotations'].delete('buzz.block.xyz/release-namespace')
File.write("#{ARGV[0]}/tenant-missing-annotation.yaml", (resources + [policy]).map(&:to_yaml).join)
RUBY
for variant in omitted empty explicit; do
  gate <"$out/tenant-$variant.yaml" >"$out/gated.yaml"
  cmp "$out/tenant-$variant.yaml" "$out/gated.yaml"
done
for variant in default wrong missing-annotation; do
  if gate <"$out/tenant-$variant.yaml" >"$out/gated.yaml" 2>"$out/gate-error"; then
    echo "expected release-namespace gate to reject $variant" >&2
    exit 1
  fi
  test ! -s "$out/gated.yaml"
  grep -q 'external policy' "$out/gate-error"
done

# Valid parent YAML aliases in unrelated resources must pass through unchanged.
cat "$out/combined.yaml" >"$out/aliases.yaml"
cat >>"$out/aliases.yaml" <<'YAML'
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: unrelated
data:
  first: &shared "value"
  second: *shared
  date: 2026-10-06
  timestamp: 2026-10-06T16:00:00Z
YAML
gate <"$out/aliases.yaml" >"$out/gated.yaml"
cmp "$out/aliases.yaml" "$out/gated.yaml"
# Standard YAML scalars must not enable arbitrary Ruby object construction.
printf '%s\n' '--- !ruby/object:Object {}' >"$out/ruby-object.yaml"
if gate <"$out/ruby-object.yaml" >"$out/gated.yaml" 2>"$out/gate-error"; then
  echo 'expected Ruby object tag to be rejected' >&2
  exit 1
fi
test ! -s "$out/gated.yaml"
grep -q 'DisallowedClass' "$out/gate-error"

# Helm 3 excludes hooks from post-renderer stdin. The complete template stream
# must reject a hook colliding with an otherwise valid ordinary replacement.
mkdir -p "$out/parent/templates"
printf 'apiVersion: v2\nname: policy-hook-fixture\nversion: 0.1.0\n' >"$out/parent/Chart.yaml"
cp "$out/combined.yaml" "$out/parent/templates/runtime.yaml"
cat >"$out/parent/templates/hook.yaml" <<'YAML'
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: platform-runtime
  annotations:
    helm.sh/hook: pre-upgrade
    helm.sh/hook-delete-policy: before-hook-creation,hook-succeeded
spec:
  podSelector: {}
  policyTypes: [Ingress, Egress]
YAML
if [[ $(helm version --short) == v3.* ]]; then
  helm template push "$out/parent" --is-upgrade --post-renderer "$chart/tests/check-external-policy.rb" >"$out/post-rendered.yaml"
fi
if helm template push "$out/parent" --is-upgrade | gate >"$out/gated.yaml" 2>"$out/gate-error"; then
  echo 'expected complete render to reject a hook collision' >&2
  exit 1
fi
test ! -s "$out/gated.yaml"
grep -q 'external policy' "$out/gate-error"

# External-mode callers must bind expectations independently of the marker.
env -u GEM_HOME -u GEM_PATH -u RUBYLIB -u RUBYOPT ruby -ryaml - "$out" <<'RUBY'
resources = YAML.load_stream(File.read("#{ARGV[0]}/combined.yaml")).compact
pod = resources.find { |r| r['kind'] == 'Deployment' }
pod['metadata'].delete('annotations')
File.write("#{ARGV[0]}/missing-marker.yaml", resources.map(&:to_yaml).join)
RUBY
if env -u GEM_HOME -u GEM_PATH -u RUBYLIB -u RUBYOPT ruby "$chart/tests/check-external-policy.rb" --expect push-buzz-push-gateway platform-runtime default <"$out/missing-marker.yaml" >"$out/gated.yaml" 2>"$out/gate-error"; then
  echo 'expected strict gate to reject removed marker' >&2
  exit 1
fi
test ! -s "$out/gated.yaml"
grep -q 'missing or overwritten ownership marker' "$out/gate-error"
