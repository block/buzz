# Buzz push gateway chart

### Platform-managed runtime integration

`podLabels`, `serviceAccountName`, and `terminationGracePeriodSeconds` configure
only runtime pods. Defaults retain the existing pod specification; the grace
period cannot be shorter than 60 seconds. Additional labels cannot override the
chart's name, instance, or component selectors. Migration settings remain separate.

A parent chart that supplies its own runtime NetworkPolicy can set
`networkPolicy.enabled=false` and `networkPolicy.externalPolicyName` to that
policy's name. The name is an explicit acknowledgement, not a cluster existence
check: the parent must render and validate a policy selecting the runtime pods
before deployment. Supplying a replacement name while the upstream policy is
enabled is rejected. The migration NetworkPolicy remains enabled independently.

External-policy releases must run `tests/check-external-policy.rb` on the
**complete parent render including hooks**, before install or upgrade. Helm 3
excludes hooks from its post-renderer input, so `--post-renderer` alone is not a
sufficient gate. Render without `--no-hooks` or resource filtering, using the
same chart, release name, namespace, values and capabilities as the deployment:

```sh
set -o pipefail
helm template RELEASE PARENT_CHART --namespace NAMESPACE -f VALUES.yaml \
  | ruby tests/check-external-policy.rb --expect DEPLOYMENT POLICY NAMESPACE > validated-release.yaml
```

`--expect` is mandatory for external-policy deployment and CI invocations.
Provide the intended Deployment name, replacement policy name and release namespace
from deployment configuration, not by extracting the annotations being checked.
Repeat it for each external-policy Deployment. Missing Deployments or removed or
overwritten markers then fail closed. No-argument mode is only for renders that
intentionally have no external-policy requirement, such as default-mode tests.

For upgrade preflight, include `--is-upgrade`. Deployment automation must stop if
this pipeline fails and must not change the chart or rendering inputs between
validation and deployment. Run the post-renderer additionally during Helm
install/upgrade to check ordinary resources, but retain the full-render preflight
to detect hook collisions. GitOps render pipelines must validate the complete
hook-inclusive output before submitting it to their reconciler.
The Deployment records the required policy name. The gate rejects a missing,
misspelled, duplicate, wrong-namespace or non-selecting replacement, and requires
both ingress and egress isolation. It emits no manifests on failure and preserves
successful output byte-for-byte. Parent CI must run this same gate on its complete
render; a subchart-only render intentionally fails in external-policy mode.
The deployment owner must separately validate the allowed traffic and rolling drain.
