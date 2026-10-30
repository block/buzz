#!/usr/bin/env bash
# Prove that a cluster can resolve and start the exact digest-qualified Sprig
# image that will be passed to the Kubernetes provider. This is intentionally a
# separate preflight: a Docker image existing in the host daemon does not imply
# that the kubelet's container runtime can resolve the same name and digest.
set -euo pipefail

: "${BUZZ_K8S_TEST_CONTEXT:?set the explicit disposable/local kubectl context}"
: "${BUZZ_SPRIG_IMAGE:?set an immutable image reference (name@sha256:<64 hex>)}"

if [[ ! "$BUZZ_SPRIG_IMAGE" =~ ^[^[:space:]@]+@sha256:[0-9a-fA-F]{64}$ ]]; then
    echo "error: BUZZ_SPRIG_IMAGE must be name@sha256:<64 hex>" >&2
    exit 2
fi

CONTEXT="$BUZZ_K8S_TEST_CONTEXT"
IMAGE="$BUZZ_SPRIG_IMAGE"
PULL_POLICY="${BUZZ_K8S_TEST_PULL_POLICY:-IfNotPresent}"
case "$PULL_POLICY" in
    Always|IfNotPresent|Never) ;;
    *) echo "error: invalid BUZZ_K8S_TEST_PULL_POLICY: $PULL_POLICY" >&2; exit 2 ;;
esac

MANAGED_BY="buzz-backend-kubernetes"
BINDING_VERSION="v1"
NAMESPACE="buzz-k8s-sprig-$(date +%s)-$RANDOM"
CREATED=0

cleanup() {
    (( CREATED == 1 )) || return 0
    local managed binding foreign
    managed="$(kubectl --context "$CONTEXT" get namespace "$NAMESPACE" \
        -o jsonpath='{.metadata.labels.app\.kubernetes\.io/managed-by}' 2>/dev/null || true)"
    binding="$(kubectl --context "$CONTEXT" get namespace "$NAMESPACE" \
        -o jsonpath='{.metadata.labels.buzz\.block\.xyz/binding-version}' 2>/dev/null || true)"
    if [[ "$managed" != "$MANAGED_BY" || "$binding" != "$BINDING_VERSION" ]]; then
        echo "REFUSING cleanup: namespace ownership markers changed: $NAMESPACE" >&2
        return 1
    fi
    foreign="$(kubectl --context "$CONTEXT" --namespace "$NAMESPACE" get pods -o json \
        | jq '[.items[] | select(.metadata.labels["app.kubernetes.io/managed-by"] != "buzz-backend-kubernetes" or .metadata.labels["buzz.block.xyz/binding-version"] != "v1")] | length')"
    if [[ "$foreign" != 0 ]]; then
        echo "REFUSING cleanup: namespace contains an unowned pod: $NAMESPACE" >&2
        return 1
    fi
    kubectl --context "$CONTEXT" delete namespace "$NAMESPACE" --wait=true
}
trap cleanup EXIT

# Fail before mutation if the named context is absent or inaccessible. Print the
# cluster identity so evidence cannot be mistaken for a different kubeconfig.
kubectl config get-contexts "$CONTEXT" >/dev/null
SERVER="$(kubectl config view --minify --context "$CONTEXT" -o jsonpath='{.clusters[0].cluster.server}')"
kubectl --context "$CONTEXT" get nodes -o name >/dev/null
printf 'context=%s\nserver=%s\nimage=%s\npull_policy=%s\nnamespace=%s\n' \
    "$CONTEXT" "$SERVER" "$IMAGE" "$PULL_POLICY" "$NAMESPACE"

kubectl --context "$CONTEXT" create namespace "$NAMESPACE"
CREATED=1
kubectl --context "$CONTEXT" label namespace "$NAMESPACE" \
    "app.kubernetes.io/managed-by=$MANAGED_BY" \
    "buzz.block.xyz/binding-version=$BINDING_VERSION"

cat <<YAML | kubectl --context "$CONTEXT" --namespace "$NAMESPACE" apply -f -
apiVersion: v1
kind: Pod
metadata:
  name: digest-resolution-probe
  labels:
    app.kubernetes.io/managed-by: $MANAGED_BY
    buzz.block.xyz/binding-version: $BINDING_VERSION
spec:
  restartPolicy: Never
  containers:
    - name: probe
      image: $IMAGE
      imagePullPolicy: $PULL_POLICY
      command: [/bin/bash, -ceu]
      args:
        - 'test "\$(readlink /usr/local/bin/buzz-acp)" = sprig; echo DIGEST_ABI_OK'
YAML

if ! kubectl --context "$CONTEXT" --namespace "$NAMESPACE" wait \
    --for=jsonpath='{.status.phase}'=Succeeded pod/digest-resolution-probe --timeout=120s; then
    kubectl --context "$CONTEXT" --namespace "$NAMESPACE" describe pod digest-resolution-probe >&2 || true
    exit 1
fi

kubectl --context "$CONTEXT" --namespace "$NAMESPACE" logs digest-resolution-probe
IMAGE_ID="$(kubectl --context "$CONTEXT" --namespace "$NAMESPACE" get pod digest-resolution-probe \
    -o jsonpath='{.status.containerStatuses[0].imageID}')"
RESOLVED_SPEC="$(kubectl --context "$CONTEXT" --namespace "$NAMESPACE" get pod digest-resolution-probe \
    -o jsonpath='{.spec.containers[0].image}')"
[[ "$RESOLVED_SPEC" == "$IMAGE" ]]
[[ -n "$IMAGE_ID" ]]
printf 'resolved_spec=%s\nimage_id=%s\nPASS: exact digest-qualified Sprig reference started\n' \
    "$RESOLVED_SPEC" "$IMAGE_ID"
