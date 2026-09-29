# 12 IDENTITY SUBSCRIPTION SYNC

> Concrete local test cases for the corresponding feature. Add evidence to `testing/LOCAL_VALIDATION_LOG.md` after execution.

| ID | Test | Procedure | Expected result |
|---|---|---|---|
| TC-F12-001 | Website signup | Sign up with test email/password. | Account is created by auth server; desktop receives a one-time authorization result. |
| TC-F12-002 | Desktop session | Restart the desktop after login. | Secure session is restored without storing plaintext password. |
| TC-F12-003 | Entitlement gate | Disable cloud-sync entitlement. | Local brain continues; cloud sync is blocked. |
| TC-F12-004 | Two-device sync | Create a memory on device A and sync to device B. | B reconstructs logical memory locally. |
| TC-F12-005 | Offline transition | Disconnect network after sync. | Local recall continues without cloud connectivity. |

## Status
- [ ] Automated coverage implemented where appropriate
- [ ] Manual local validation completed
- [ ] Evidence recorded
