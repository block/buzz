#!/usr/bin/env bash
# Local docker-desktop k8s testbed for the Buzz relay mesh.
#
# Repeatable path: build image -> helm dep build -> helm install (quickstart HA,
# 3 replicas) -> wait 3/3 Ready -> probe /_readiness on every pod. This is the
# baseline every mesh build redeploys onto (mesh lane).
#
# Prereqs: docker-desktop k8s context Ready, helm >= 3.14, kubectl, docker.
# docker-desktop shares the docker image store with k8s, so a locally-built
# tag + pullPolicy: IfNotPresent needs no registry push or `kind load`.
#
# Usage:
#   deploy/local/build-and-deploy.sh                # full: build + deploy
#   SKIP_BUILD=1 deploy/local/build-and-deploy.sh   # redeploy existing image
#   IMAGE_TAG=mesh-abc1234 deploy/local/build-and-deploy.sh
set -euo pipefail

# ── config ──────────────────────────────────────────────────────────────────
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
NS="${NS:-buzz-mesh}"
RELEASE="${RELEASE:-buzz}"
IMAGE_REPO="${IMAGE_REPO:-buzz-relay}"
IMAGE_TAG="${IMAGE_TAG:-mesh-local}"
CHART="${REPO_ROOT}/deploy/charts/buzz"
VALUES="${REPO_ROOT}/deploy/local/quickstart-ha-values.yaml"
CA_PEM="${REPO_ROOT}/deploy/local/proxy-ca.pem"
EXPECT_CTX="docker-desktop"
REPLICAS=3
EVID="${EVID:-/tmp/mesh-build/deploy-evidence-$(date +%Y%m%d-%H%M%S)}"
mkdir -p "$EVID"

log() { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }
die() { printf '\033[1;31mFAIL: %s\033[0m\n' "$*" >&2; exit 1; }

# ── 0. guardrails ─────────────────────────────────────────────────────────────
CTX="$(kubectl config current-context)"
[ "$CTX" = "$EXPECT_CTX" ] || die "kube context is '$CTX', expected '$EXPECT_CTX' (refusing to touch a non-local cluster)"
log "context: $CTX"; kubectl get nodes | tee "$EVID/nodes.txt"

# ── 1. corporate-proxy CA + npm mirror (TLS-intercepting gateway) ────────────
# Two stacked blocks on Block's network: (a) the gateway re-signs TLS with
# internal CAs the build container doesn't trust; (b) public registry.npmjs.org
# is policy-blocked (Dependency Confusion mitigation), so npm/corepack must use
# the Artifactory mirror. Both no-op on a normal network (build-args stay unset).
CA_ARG=()
REG_ARG=()
# (a) Build a complete internal-CA bundle from the macOS System keychain.
if [ ! -f "$CA_PEM" ] && command -v security >/dev/null 2>&1; then
  log "exporting Block internal CA bundle from System keychain"
  : > "$CA_PEM"
  for name in "Cloudflare Gateway CA" \
              "Service To Service AWS Native CA production G0" \
              "Corp Systems AWS Native CA production G0" \
              "Block, Inc CA G1" \
              "Square Primary Certificate Authority - G2"; do
    security find-certificate -a -c "$name" -p /Library/Keychains/System.keychain >> "$CA_PEM" 2>/dev/null || true
  done
fi
if [ -f "$CA_PEM" ] && grep -q 'BEGIN CERTIFICATE' "$CA_PEM"; then
  CA_ARG=(--build-arg "EXTRA_CA_CERTS=deploy/local/proxy-ca.pem")
  log "using proxy CA bundle ($(grep -c 'BEGIN CERTIFICATE' "$CA_PEM") certs)"
fi
# (b) Use the host's configured npm registry (Artifactory) if it isn't public.
HOST_NPM_REG="$(pnpm config get registry 2>/dev/null || echo '')"
if [ -n "$HOST_NPM_REG" ] && ! echo "$HOST_NPM_REG" | grep -q 'registry.npmjs.org'; then
  REG_ARG=(--build-arg "NPM_REGISTRY=${HOST_NPM_REG}")
  log "using npm registry mirror: $HOST_NPM_REG"
fi

# ── 2. build image ────────────────────────────────────────────────────────────
if [ "${SKIP_BUILD:-0}" != "1" ]; then
  log "building ${IMAGE_REPO}:${IMAGE_TAG} (HEAD $(git -C "$REPO_ROOT" rev-parse --short HEAD))"
  git -C "$REPO_ROOT" rev-parse HEAD > "$EVID/build-sha.txt"
  docker build "${CA_ARG[@]}" "${REG_ARG[@]}" \
    -t "${IMAGE_REPO}:${IMAGE_TAG}" \
    -f "${REPO_ROOT}/Dockerfile" "${REPO_ROOT}" 2>&1 | tee "$EVID/build.log"
else
  log "SKIP_BUILD=1 — reusing ${IMAGE_REPO}:${IMAGE_TAG}"
fi
docker image inspect "${IMAGE_REPO}:${IMAGE_TAG}" --format '{{.Id}} {{.Size}}' | tee "$EVID/image-id.txt"

# ── 3. chart deps + install ───────────────────────────────────────────────────
log "helm dependency build"
helm dependency build "$CHART" 2>&1 | tee "$EVID/helm-dep.txt"

log "helm upgrade --install $RELEASE (ns=$NS, replicas=$REPLICAS)"
# No --wait here: the relay's A3 S3 probe is startup-fatal, so relays
# CrashLoopBackOff a few times until the concurrent init Job creates the bucket.
# helm --wait races that transient and can bail early; instead we own readiness
# gating below (rollout status + per-pod probe), which tolerates the restarts.
helm upgrade --install "$RELEASE" "$CHART" \
  --namespace "$NS" --create-namespace \
  --values "$VALUES" \
  --set image.repository="$IMAGE_REPO" \
  --set image.tag="$IMAGE_TAG" \
  --timeout 5m 2>&1 | tee "$EVID/helm-install.txt"
helm_rc=${PIPESTATUS[0]}
if [ "$helm_rc" != 0 ]; then
  kubectl -n "$NS" get pods -o wide | tee "$EVID/pods-onfail.txt"
  kubectl -n "$NS" describe pods -l app.kubernetes.io/name=buzz | tee "$EVID/describe-onfail.txt"
  kubectl -n "$NS" logs -l app.kubernetes.io/name=buzz --tail=100 --all-containers | tee "$EVID/logs-onfail.txt"
  die "helm install returned rc=$helm_rc"
fi

# ── 4. verify 3/3 Ready ───────────────────────────────────────────────────────
# Find the relay Deployment: everything under this release named "buzz" except
# the bundled "*-minio" Deployment. (The chart fullname collapses
# "<release>-<chart>" to "<release>" when the release name already contains the
# chart name, so the name isn't always "<release>-buzz".)
DEPLOY=""
for d in $(kubectl -n "$NS" get deploy -l "app.kubernetes.io/instance=$RELEASE" \
             -o jsonpath='{range .items[*]}{.metadata.name}{"\n"}{end}'); do
  case "$d" in *-minio) continue;; esac
  DEPLOY="$d"; break
done
[ -n "$DEPLOY" ] || die "could not locate the relay Deployment"
log "waiting for $REPLICAS relay pods Ready (deployment: $DEPLOY)"
kubectl -n "$NS" rollout status deployment/"$DEPLOY" --timeout=4m | tee "$EVID/rollout.txt"
kubectl -n "$NS" get pods -o wide | tee "$EVID/pods.txt"

READY=$(kubectl -n "$NS" get deploy "$DEPLOY" -o jsonpath='{.status.readyReplicas}')
[ "${READY:-0}" = "$REPLICAS" ] || die "readyReplicas=$READY, expected $REPLICAS"
log "deployment reports $READY/$REPLICAS Ready"

# ── 5. probe /_readiness on EVERY relay pod (not just the deployment aggregate)
# The bundled MinIO + init pods share app.kubernetes.io/name=buzz, so select by
# the relay Deployment's own pod-template hash to hit only relay pods.
log "probing /_readiness on each relay pod individually"
: > "$EVID/readiness.txt"
FAIL=0
RELAY_PODS=$(kubectl -n "$NS" get pods \
  -l "app.kubernetes.io/name=buzz,app.kubernetes.io/instance=$RELEASE" \
  -o jsonpath='{range .items[*]}{.metadata.name}{" "}{.metadata.labels.app\.kubernetes\.io/component}{"\n"}{end}' \
  | awk '$2 != "minio" && $2 != "minio-init" {print $1}')
for pod in $RELAY_PODS; do
  body=$(kubectl -n "$NS" exec "$pod" -- \
    sh -c 'curl -sS --max-time 5 http://127.0.0.1:8080/_readiness' 2>/dev/null || echo '<curl-failed>')
  echo "$pod -> $body" | tee -a "$EVID/readiness.txt"
  echo "$body" | grep -q '"status":"ready"' || FAIL=1
done
[ "$FAIL" = 0 ] || die "at least one pod is not reporting ready (see $EVID/readiness.txt)"

log "ALL $REPLICAS PODS READY — baseline up. Evidence: $EVID"
echo "namespace=$NS release=$RELEASE image=${IMAGE_REPO}:${IMAGE_TAG}" | tee "$EVID/SUMMARY.txt"
