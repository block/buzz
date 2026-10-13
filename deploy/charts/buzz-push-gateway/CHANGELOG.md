# Chart release notes

## 0.4.0

- Replace `profiles.dogfood` with `application` and use unprefixed application/APNs environment variables. Requires the matching buzz-push-v1 gateway image and fresh authority store.

## 0.3.5

Adds optional string-valued `deploymentAnnotations` for Deployment metadata,
including secret-reloader configuration. Chart-owned policy annotations remain
reserved. Runtime Pod and migration annotations are unchanged, and the empty
default preserves existing manifests.

Implemented in [#8136](https://github.com/block/buzz/pull/8136).

## 0.3.4

Adds runtime Pod labels, a service account name, and a configurable termination
grace period of at least 60 seconds. Reserved selector labels cannot be overridden.

Allows a platform to replace the runtime NetworkPolicy by explicitly naming its
replacement. External policy mode supports PodMonitor without requiring unused
chart-owned monitoring selectors or APNs destinations. The complete-render gate
requires exactly one ordinary replacement NetworkPolicy
in the effective release namespace, selecting the runtime Pods and declaring
both ingress and egress policy types. Helm and Argo CD hook policies are rejected.
External-policy callers must supply independent Deployment, policy and namespace
expectations so removed or overwritten markers fail closed. The gate validates
selector expression grammar and expands Kubernetes Lists. Operators must validate
the full parent render including hooks; a Helm 3 post-renderer alone is insufficient.

Migration NetworkPolicy remains chart-owned and continues to require database
and DNS destinations. Runtime settings do not change migration Pod identity.
Default runtime rendering is unchanged.

Implemented in [#8113](https://github.com/block/buzz/pull/8113).

## 0.3.3

`gatewayOrigin` is now only an HTTPRoute routing input. It is optional unless
`httpRoute.enabled=true`, and is no longer passed to the gateway container as
`BUZZ_PUSH_GATEWAY_ORIGIN`. Implemented in
[#7877](https://github.com/block/buzz/pull/7877).

Adds optional `migration.podAnnotations` for the migration Job's Pod template.
The default is empty. Annotation values must be strings, and migration Pod
annotations are independent of runtime Deployment Pod annotations and Job hook
metadata.

For example, operators can set `migration.podAnnotations.sidecar.istio.io/inject`
to the string `"false"` to opt migration Pods out of Istio sidecar injection.

Implemented in [#8105](https://github.com/block/buzz/pull/8105).
