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
