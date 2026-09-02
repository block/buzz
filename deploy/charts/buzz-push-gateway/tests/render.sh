#!/usr/bin/env bash
set -euo pipefail
out=$(mktemp); production_out=$(mktemp); route_out=$(mktemp); datadog_out=$(mktemp)
trap 'rm -f "$out" "$production_out" "$route_out" "$datadog_out" "${monitoring_out:-}"' EXIT

# Defaults must lint and render without parameter injection.
helm lint deploy/charts/buzz-push-gateway >/dev/null
helm template push deploy/charts/buzz-push-gateway >"$out"
# Production values support a platform-owned ingress without rendering an
# HTTPRoute. The environment-owned inputs remain mandatory.
production_args=(
  -f deploy/charts/buzz-push-gateway/values-production.yaml
  --set 'image.digest=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
  --set 'profiles.dogfood.appAttestAppId=REALTEAM.xyz.block.buzz.dogfood.mobile'
  --set 'networkPolicy.postgresEgressCidrs[0]=10.42.0.0/16'
)
helm lint deploy/charts/buzz-push-gateway "${production_args[@]}" >/dev/null
helm template push deploy/charts/buzz-push-gateway "${production_args[@]}" >"$production_out"

# Gateway API remains an explicit supported ingress mode when an operator opts
# in and supplies the environment-owned parent.
helm template push deploy/charts/buzz-push-gateway \
  --set httpRoute.enabled=true \
  --set 'httpRoute.parentRefs[0].name=production-gateway' \
  --set 'httpRoute.parentRefs[0].namespace=gateway-system' \
  >"$route_out"

env -u GEM_HOME -u GEM_PATH -u RUBYLIB -u RUBYOPT ruby -ryaml -rset \
  - "$out" "$production_out" "$route_out" <<'RUBY'
def assert!(condition, detail = "assertion failed")
  raise detail unless condition
end

xs = YAML.load_stream(File.read(ARGV[0])).compact
svc = xs.find { |x| x["kind"] == "Service" }
assert!(svc.dig("spec", "ports").map { |port| port["targetPort"] } == ["public"])
d = xs.find { |x| x["kind"] == "Deployment" }
j = xs.find { |x| x["kind"] == "Job" }
runtime = {
  "app.kubernetes.io/name" => "buzz-push-gateway",
  "app.kubernetes.io/instance" => "push",
  "app.kubernetes.io/component" => "runtime",
}
migration = runtime.merge("app.kubernetes.io/component" => "migration")
assert!(svc.dig("spec", "selector") == runtime)
assert!(d.dig("spec", "selector", "matchLabels") == runtime)
assert!(d.dig("spec", "template", "metadata", "labels") == runtime)
assert!(d.dig("spec", "template", "metadata", "annotations").nil?)
assert!(j.dig("spec", "template", "metadata", "labels") == migration)
assert!(svc.dig("spec", "selector") != j.dig("spec", "template", "metadata", "labels"))
jenv = j.dig("spec", "template", "spec", "containers", 0, "env").to_h { |entry| [entry["name"], entry] }
assert!(jenv.dig("BUZZ_PUSH_RUNTIME_DATABASE_ROLE", "value") == "buzz_push_gateway_runtime")
assert!(jenv.fetch("DATABASE_URL").key?("valueFrom"))
assert!(j.dig("spec", "template", "spec", "containers", 0, "args") == ["--migrate-only"])
assert!(j.dig("metadata", "annotations") == {
  "helm.sh/hook" => "pre-install,pre-upgrade",
  "helm.sh/hook-weight" => "-5",
  "helm.sh/hook-delete-policy" => "before-hook-creation,hook-succeeded",
})
env_names = d.dig("spec", "template", "spec", "containers", 0, "env")
  .map { |entry| entry["name"] }.to_set
required = Set.new(%w[
  DATABASE_URL BUZZ_PUSH_DOGFOOD_APNS_CERT_PATH
  BUZZ_PUSH_DOGFOOD_APNS_TOPIC BUZZ_PUSH_DOGFOOD_APP_ATTEST_APP_ID
  BUZZ_PUSH_GRANT_KEYS BUZZ_PUSH_TOKEN_KEYS BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS
])
assert!(required.subset?(env_names))
assert!(!env_names.any? { |name| name.include?("APP_STORE") })
apns_volume = d.dig("spec", "template", "spec", "volumes").find { |volume| volume["name"] == "apns-dogfood" }
assert!(apns_volume.dig("secret", "defaultMode") == 0o400, apns_volume.inspect)
assert!(d.dig("spec", "replicas") >= 2)
assert!(!xs.any? { |x| x["kind"] == "HTTPRoute" })
# Observability is opt-in: default render exposes no scrape CRDs and 8081 stays
# free of pod ingress (only 8080 is reachable).
assert!(!xs.any? { |x| %w[PodMonitor PrometheusRule].include?(x["kind"]) })
nps = xs.select { |x| x["kind"] == "NetworkPolicy" }
np = nps.find { |x| x.dig("metadata", "name") == "push-buzz-push-gateway" }
migration_np = nps.find { |x| x.dig("metadata", "name") == "push-buzz-push-gateway-migration" }
assert!(np.dig("spec", "podSelector", "matchLabels") == runtime)
assert!(migration_np.dig("spec", "podSelector", "matchLabels") == migration)
assert!(migration_np.dig("metadata", "annotations") == {
  "helm.sh/hook" => "pre-install,pre-upgrade",
  "helm.sh/hook-weight" => "-10",
  "helm.sh/hook-delete-policy" => "before-hook-creation",
})
assert!(migration_np.dig("metadata", "annotations", "helm.sh/hook-weight").to_i < j.dig("metadata", "annotations", "helm.sh/hook-weight").to_i)
assert!(migration_np.dig("spec", "ingress") == [])
assert!(migration_np.dig("spec", "policyTypes") == %w[Ingress Egress])
migration_ports = migration_np.dig("spec", "egress")
  .flat_map { |rule| rule.fetch("ports", []) }.map { |port| port["port"] }.to_set
assert!(migration_ports == Set[53, 5432], migration_ports.inspect)
assert!(!migration_np.dig("spec", "egress").flat_map { |rule| rule.fetch("ports", []) }.any? { |port| port["port"] == 443 })
ingress_ports = np.dig("spec", "ingress")
  .flat_map { |rule| rule.fetch("ports", []) }.map { |port| port["port"] }.to_set
assert!(ingress_ports == Set[8080], ingress_ports.inspect)
production = YAML.load_stream(File.read(ARGV[1])).compact
assert!(!production.any? { |x| x["kind"] == "HTTPRoute" })
production_deployment = production.find { |x| x["kind"] == "Deployment" }
production_image = production_deployment.dig("spec", "template", "spec", "containers", 0, "image")
assert!(production_image == "ghcr.io/block/buzz-push-gateway@sha256:#{"a" * 64}", production_image.inspect)
route = YAML.load_stream(File.read(ARGV[2])).compact.find { |x| x["kind"] == "HTTPRoute" }
assert!(!route.dig("spec", "parentRefs").empty?)
assert!(route.dig("spec", "hostnames").include?("push.buzz.xyz"))
RUBY

# Legacy token-auth values must fail rather than silently selecting the default
# certificate Secret.
if helm template push deploy/charts/buzz-push-gateway \
  --set apnsKey.secretName=legacy-apns-secret \
  --set apnsKey.secretKey=legacy-provider.p8 >/dev/null 2>&1; then
  echo 'expected legacy apnsKey values to fail schema validation' >&2
  exit 1
fi

# Enabling a route without a Gateway attachment must fail schema validation.
if helm template push deploy/charts/buzz-push-gateway --set httpRoute.enabled=true >/dev/null 2>&1; then
  echo 'expected httpRoute.enabled=true without parentRefs to fail' >&2
  exit 1
fi

# The checked-in production contract is intentionally undeployable until CI or
# the release system supplies its environment-owned values.
if helm template push deploy/charts/buzz-push-gateway -f deploy/charts/buzz-push-gateway/values-production.yaml >/dev/null 2>&1; then
  echo 'expected uninjected production values to fail' >&2
  exit 1
fi

# Enabling observability renders the scrape CRDs and adds a scoped 8081 ingress
# keyed to the named monitoring source — never a blanket 8081 rule.
monitoring_out=$(mktemp)
helm template push deploy/charts/buzz-push-gateway \
  --set podMonitor.enabled=true \
  --set prometheusRule.enabled=true \
  --set networkPolicy.monitoring.enabled=true \
  --set 'networkPolicy.monitoring.namespaceSelector.kubernetes\.io/metadata\.name=monitoring' \
  --set 'networkPolicy.monitoring.podSelector.app\.kubernetes\.io/name=prometheus' \
  >"$monitoring_out"

env -u GEM_HOME -u GEM_PATH -u RUBYLIB -u RUBYOPT ruby -ryaml -rset \
  - "$monitoring_out" <<'RUBY'
def assert!(condition, detail = "assertion failed")
  raise detail unless condition
end

xs = YAML.load_stream(File.read(ARGV[0])).compact
pm = xs.find { |x| x["kind"] == "PodMonitor" }
endpoint = pm.dig("spec", "podMetricsEndpoints", 0)
assert!(endpoint["port"] == "health" && endpoint["path"] == "/metrics", endpoint.inspect)
assert!(!xs.find { |x| x["kind"] == "PrometheusRule" }.dig("spec", "groups").empty?)
np = xs.find do |x|
  x["kind"] == "NetworkPolicy" && x.dig("metadata", "name") == "push-buzz-push-gateway"
end
monitoring = np.dig("spec", "ingress").select do |rule|
  rule.fetch("ports", []).map { |port| port["port"] }.to_set == Set[8081]
end
assert!(monitoring.length == 1, "exactly one scoped 8081 ingress rule")
from = monitoring[0].fetch("from")[0]
# 8081 ingress must be scoped by both selectors, never empty/blanket.
assert!(!from.dig("namespaceSelector", "matchLabels").empty? && !from.dig("podSelector", "matchLabels").empty?, from.inspect)
RUBY

# Datadog discovers the same private endpoint from pod annotations and needs no
# prometheus-operator CRDs. Its agent ingress remains selector-scoped.
helm lint deploy/charts/buzz-push-gateway \
  -f deploy/charts/buzz-push-gateway/tests/datadog-values.yaml >/dev/null
helm template push deploy/charts/buzz-push-gateway \
  -f deploy/charts/buzz-push-gateway/tests/datadog-values.yaml \
  >"$datadog_out"

env -u GEM_HOME -u GEM_PATH -u RUBYLIB -u RUBYOPT ruby -rjson -ryaml -rset \
  - "$datadog_out" <<'RUBY'
def assert!(condition, detail = "assertion failed")
  raise detail unless condition
end

xs = YAML.load_stream(File.read(ARGV[0])).compact
assert!(!xs.any? { |x| %w[PodMonitor PrometheusRule].include?(x["kind"]) })
deployment = xs.find { |x| x["kind"] == "Deployment" }
raw_check = deployment.dig(
  "spec", "template", "metadata", "annotations",
  "ad.datadoghq.com/gateway.checks",
)
check = JSON.parse(raw_check)
instance = check.dig("openmetrics", "instances", 0)
assert!(instance["openmetrics_endpoint"] == "http://%%host%%:8081/metrics", instance.inspect)
assert!(instance["metrics"] == ["push_gateway_.*"], instance.inspect)

np = xs.find do |x|
  x["kind"] == "NetworkPolicy" && x.dig("metadata", "name") == "push-buzz-push-gateway"
end
monitoring = np.dig("spec", "ingress").select do |rule|
  rule.fetch("ports", []).map { |port| port["port"] }.to_set == Set[8081]
end
assert!(monitoring.length == 1, "exactly one scoped Datadog 8081 ingress rule")
from = monitoring[0].fetch("from")[0]
assert!(!from.dig("namespaceSelector", "matchLabels").empty?, from.inspect)
assert!(!from.dig("podSelector", "matchLabels").empty?, from.inspect)
RUBY

# Negative: monitoring enabled with default empty selectors must fail (would
# otherwise render a blanket 8081 rule matching all namespaces/pods).
if helm template push deploy/charts/buzz-push-gateway \
  --set podMonitor.enabled=true \
  --set networkPolicy.monitoring.enabled=true >/dev/null 2>&1; then
  echo 'expected monitoring.enabled with empty selectors to fail' >&2
  exit 1
fi

# Negative: PodMonitor without ingress is an unreachable scraper and must fail.
# Scoped ingress without PodMonitor is valid for annotation-discovered agents.
if helm template push deploy/charts/buzz-push-gateway \
  --set podMonitor.enabled=true \
  --set 'networkPolicy.monitoring.namespaceSelector.kubernetes\.io/metadata\.name=monitoring' \
  --set 'networkPolicy.monitoring.podSelector.app\.kubernetes\.io/name=prometheus' \
  >/dev/null 2>&1; then
  echo 'expected podMonitor.enabled without monitoring ingress to fail' >&2
  exit 1
fi

# Negative: retry-ratio threshold is a fraction; a value > 1 must fail schema.
if helm template push deploy/charts/buzz-push-gateway \
  --set prometheusRule.enabled=true \
  --set prometheusRule.apnsRetryRatioThreshold=2 >/dev/null 2>&1; then
  echo 'expected apnsRetryRatioThreshold=2 to fail' >&2
  exit 1
fi
