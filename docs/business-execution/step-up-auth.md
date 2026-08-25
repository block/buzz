# Step-up Authentication

Workbench login is not Step-up. A future pilot must define and test:

- required Authentik authentication context and MFA factor;
- when final approval triggers reauthentication;
- maximum Step-up age and expiry behavior;
- cryptographic/session binding from the Step-up result to one approval decision;
- revocation when user, binding, BusinessSession or policy changes.

The repository has an Authentik OIDC/PKCE and Embed Session POC but no
action-specific Step-up flow or runtime evidence. `step_up_auth_ready=false`.
