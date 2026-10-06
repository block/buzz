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

External-policy releases must run `tests/check-external-policy.rb` as a Helm
post-renderer on the **complete parent release**, before install or upgrade.
The Deployment records the required policy name. The gate rejects a missing,
misspelled, duplicate, wrong-namespace or non-selecting replacement, and requires
both ingress and egress isolation. It emits no manifests on failure and preserves
successful output byte-for-byte. Parent CI must run this same gate on its complete
render; a subchart-only render intentionally fails in external-policy mode.
The deployment owner must separately validate the allowed traffic and rolling drain.
