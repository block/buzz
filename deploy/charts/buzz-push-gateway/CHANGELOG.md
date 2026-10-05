# Chart release notes

## 0.3.3

Adds optional `migration.podAnnotations` for the migration Job's Pod template.
The default is empty. Annotation values must be strings, and migration Pod
annotations are independent of runtime Deployment Pod annotations and Job hook
metadata.

For example, operators can set `migration.podAnnotations.sidecar.istio.io/inject`
to the string `"false"` to opt migration Pods out of Istio sidecar injection.

Implemented in [#8105](https://github.com/block/buzz/pull/8105).
