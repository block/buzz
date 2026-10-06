## 0.3.4

Adds runtime Pod labels, a service account name, and a configurable termination
grace period of at least 60 seconds. Reserved selector labels cannot be overridden.

Allows a platform to replace the runtime NetworkPolicy by explicitly naming its
replacement. External policy mode supports PodMonitor without requiring unused
chart-owned monitoring selectors or APNs destinations. The complete-render gate requires exactly one ordinary replacement NetworkPolicy
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
