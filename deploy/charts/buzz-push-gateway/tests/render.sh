#!/usr/bin/env bash
set -euo pipefail
out=$(mktemp); production_out=$(mktemp)
trap 'rm -f "$out" "$production_out"' EXIT

# Defaults must lint and render without parameter injection.
helm lint deploy/charts/buzz-push-gateway >/dev/null
helm template push deploy/charts/buzz-push-gateway >"$out"
# Production values must attach push.buzz.xyz to an explicit Gateway.
production_args=(
  -f deploy/charts/buzz-push-gateway/values-production.yaml
  --set 'image.digest=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
  --set 'appAttestAppId=REALTEAM.xyz.buzz'
  --set 'httpRoute.parentRefs[0].name=production-gateway'
  --set 'httpRoute.parentRefs[0].namespace=gateway-system'
  --set 'networkPolicy.postgresEgressCidrs[0]=10.42.0.0/16'
)
helm lint deploy/charts/buzz-push-gateway "${production_args[@]}" >/dev/null
helm template push deploy/charts/buzz-push-gateway "${production_args[@]}" >"$production_out"

python3 - "$out" "$production_out" <<'PY'
import sys,yaml
xs=list(yaml.safe_load_all(open(sys.argv[1])))
svc=next(x for x in xs if x and x.get('kind')=='Service')
assert [p['targetPort'] for p in svc['spec']['ports']]==['public']
d=next(x for x in xs if x and x.get('kind')=='Deployment')
env={e['name'] for e in d['spec']['template']['spec']['containers'][0]['env']}
required={'DATABASE_URL','BUZZ_PUSH_APNS_KEY_ID','BUZZ_PUSH_APNS_TEAM_ID','BUZZ_PUSH_APNS_TOPIC','BUZZ_PUSH_GRANT_KEYS','BUZZ_PUSH_TOKEN_KEYS','BUZZ_PUSH_MAX_GRANT_LIFETIME_SECONDS'}
assert required <= env
assert d['spec']['replicas'] >= 2
assert not any(x and x.get('kind')=='HTTPRoute' for x in xs)
# Observability is opt-in: default render exposes no scrape CRDs and 8081 stays
# free of pod ingress (only 8080 is reachable).
assert not any(x and x.get('kind') in ('PodMonitor','PrometheusRule') for x in xs)
np=next(x for x in xs if x and x.get('kind')=='NetworkPolicy')
ingress_ports={p['port'] for rule in np['spec']['ingress'] for p in rule.get('ports',[])}
assert ingress_ports=={8080}, ingress_ports
production=list(yaml.safe_load_all(open(sys.argv[2])))
route=next(x for x in production if x and x.get('kind')=='HTTPRoute')
assert route['spec']['parentRefs']
assert 'push.buzz.xyz' in route['spec']['hostnames']
PY

# Enabling a route without a Gateway attachment must fail schema validation.
if helm template push deploy/charts/buzz-push-gateway --set httpRoute.enabled=true >/dev/null 2>&1; then
  echo 'expected httpRoute.enabled=true without parentRefs to fail' >&2
  exit 1
fi

# The checked-in production contract is intentionally undeployable until CI or
# the release system supplies an immutable digest and environment-owned values.
if helm template push deploy/charts/buzz-push-gateway -f deploy/charts/buzz-push-gateway/values-production.yaml >/dev/null 2>&1; then
  echo 'expected uninjected production values to fail' >&2
  exit 1
fi

# Enabling observability renders the scrape CRDs and adds a scoped 8081 ingress
# keyed to the named monitoring source — never a blanket 8081 rule.
monitoring_out=$(mktemp); trap 'rm -f "$out" "$production_out" "$monitoring_out"' EXIT
helm template push deploy/charts/buzz-push-gateway \
  --set podMonitor.enabled=true \
  --set prometheusRule.enabled=true \
  --set networkPolicy.monitoring.enabled=true \
  --set 'networkPolicy.monitoring.namespaceSelector.kubernetes\.io/metadata\.name=monitoring' \
  --set 'networkPolicy.monitoring.podSelector.app\.kubernetes\.io/name=prometheus' \
  >"$monitoring_out"

python3 - "$monitoring_out" <<'PY'
import sys,yaml
xs=list(yaml.safe_load_all(open(sys.argv[1])))
pm=next(x for x in xs if x and x.get('kind')=='PodMonitor')
ep=pm['spec']['podMetricsEndpoints'][0]
assert ep['port']=='health' and ep['path']=='/metrics', ep
assert next(x for x in xs if x and x.get('kind')=='PrometheusRule')['spec']['groups']
np=next(x for x in xs if x and x.get('kind')=='NetworkPolicy')
mon=[r for r in np['spec']['ingress'] if {p['port'] for p in r.get('ports',[])}=={8081}]
assert len(mon)==1, 'exactly one scoped 8081 ingress rule'
frm=mon[0]['from'][0]
# 8081 ingress must be scoped by both selectors, never empty/blanket.
assert frm['namespaceSelector']['matchLabels'] and frm['podSelector']['matchLabels'], frm
PY

# Negative: monitoring enabled with default empty selectors must fail (would
# otherwise render a blanket 8081 rule matching all namespaces/pods).
if helm template push deploy/charts/buzz-push-gateway \
  --set podMonitor.enabled=true \
  --set networkPolicy.monitoring.enabled=true >/dev/null 2>&1; then
  echo 'expected monitoring.enabled with empty selectors to fail' >&2
  exit 1
fi

# Negative: scrape flags must be coupled. PodMonitor without ingress = an
# unreachable scraper; ingress without a PodMonitor = an open hole with no
# scraper. Both mismatches must fail schema validation.
if helm template push deploy/charts/buzz-push-gateway \
  --set podMonitor.enabled=true \
  --set 'networkPolicy.monitoring.namespaceSelector.kubernetes\.io/metadata\.name=monitoring' \
  --set 'networkPolicy.monitoring.podSelector.app\.kubernetes\.io/name=prometheus' \
  >/dev/null 2>&1; then
  echo 'expected podMonitor.enabled without monitoring ingress to fail' >&2
  exit 1
fi
if helm template push deploy/charts/buzz-push-gateway \
  --set networkPolicy.monitoring.enabled=true \
  --set 'networkPolicy.monitoring.namespaceSelector.kubernetes\.io/metadata\.name=monitoring' \
  --set 'networkPolicy.monitoring.podSelector.app\.kubernetes\.io/name=prometheus' \
  >/dev/null 2>&1; then
  echo 'expected monitoring ingress without podMonitor.enabled to fail' >&2
  exit 1
fi

# Negative: retry-ratio threshold is a fraction; a value > 1 must fail schema.
if helm template push deploy/charts/buzz-push-gateway \
  --set prometheusRule.enabled=true \
  --set prometheusRule.apnsRetryRatioThreshold=2 >/dev/null 2>&1; then
  echo 'expected apnsRetryRatioThreshold=2 to fail' >&2
  exit 1
fi
