# ORBIT Authentication Server Architecture

> Dedicated hosted control-plane design for account signup/login, subscriptions, deep-link authorization, device registration and enterprise workspace access.

## 1. Purpose

The ORBIT Auth Server is a separate hosted service that provides identity and product access for the Tauri desktop application and future mobile clients.

It does **not** store the ORBIT brain by default.

Its job is to answer:

- Who is this user?
- Is their email verified?
- Which subscription/entitlements are active?
- Which organizations/workspaces may this user access?
- Which devices are registered?
- Is this device revoked?
- Is this desktop authorization request valid?

## 2. Desktop entry states

```text
First launch
   │
   ├── Log In ───────────┐
   ├── Sign Up ──────────┤ → system browser → auth.orbit.example
   └── Continue Local     │
                          ↓
                    account session
                          ↓
                    device register
                          ↓
                  optional cloud sync
```

A valid existing session may be restored from the operating-system keychain without showing the login page every time.

## 3. Core services

### Auth API

Handles:

- email/password signup
- password verification
- email verification
- password recovery
- session refresh/revocation
- account profile

### Authorization service

Handles:

- organization membership
- workspace roles
- project access
- device status
- sync entitlement
- hosted-processing entitlement

### Device registry

Tracks:

- `device_id`
- account/user ID
- public key
- platform
- OS/app version
- device label
- first/last seen
- revoke status
- optional purpose-limited device-risk fingerprint

Do not use a raw MAC address as a password-equivalent credential or the sole device identity. Prefer a random installation ID plus cryptographic device keys.

### Entitlement service

Examples:

```text
LOCAL
CLOUD_SYNC
PRO
TEAM
ENTERPRISE
HOSTED_PROCESSING
```

Entitlements are server-authoritative, with a short-lived signed local cache so temporary network loss does not immediately stop local operation.

## 4. Desktop authorization protocol

```text
1. Desktop generates state + PKCE verifier
2. Desktop opens browser
3. User signs in or signs up
4. Auth service completes account/entitlement checks
5. Browser redirects using ORBIT deep link / app link
6. Callback contains only a short-lived authorization code
7. Desktop verifies state
8. Desktop exchanges code for session credentials
9. Desktop registers/refreshes device
10. Desktop opens the authorized workspace
```

Do not put long-lived access/refresh tokens in URLs.

## 5. Suggested control-plane records

### `accounts`

```text
id
email
email_verified_at
status
created_at
updated_at
```

### `auth_credentials`

```text
account_id
password_hash
credential_version
updated_at
```

### `sessions`

```text
session_id
account_id
device_id
issued_at
expires_at
revoked_at
```

### `devices`

```text
device_id
account_id
public_key
platform
os_version
app_version
label
risk_fingerprint_hash (optional)
last_seen_at
revoked_at
created_at
```

### `organizations`

```text
organization_id
name
status
policy_id
created_at
```

### `memberships`

```text
organization_id
account_id
role
status
created_at
revoked_at
```

### `workspaces`

```text
workspace_id
owner_type
owner_id
organization_id
name
workspace_type
policy_id
created_at
```

The Auth Server should store references to enterprise workspaces, not the contents of the ORBIT memory graph.

## 6. User/device tracking boundary

The service may record product lifecycle events for security and product operations:

```text
account_created
login_success
login_failed
email_verified
device_registered
device_revoked
subscription_changed
sync_started
sync_completed
sync_failed
```

Telemetry should be content-free by default.

Do not transmit as analytics:

- source code
- prompts
- memory contents
- embeddings
- MCP payloads
- raw file paths
- raw MAC addresses

For enterprise customers, optional analytics can be disabled or restricted by policy.

## 7. Workspace authorization

Authorization is hierarchical:

```text
Account
  ↓
Organization
  ↓
Workspace
  ↓
Project / repository
  ↓
Memory / source / event
```

Every memory API request coming from a signed-in desktop should carry enough scope information for the memory layer to evaluate ownership and policy.

Authentication alone is never sufficient authorization to retrieve memory.

## 8. Separation from the memory plane

```text
        ORBIT AUTH SERVER
        ├── accounts
        ├── sessions
        ├── devices
        ├── subscriptions
        ├── organizations
        └── memberships
                 │
                 │ identity / entitlement / policy context
                 ▼
        ORBIT MEMORY DATA PLANE
        ├── PostgreSQL / SQLite
        ├── vectors
        ├── graph
        ├── source objects
        └── memory/event state
```

This separation allows ORBIT to change its authentication provider later without changing the memory schema.

## 9. Security requirements

- Passwords are processed only by the hosted authentication service.
- Desktop credentials are stored in the operating-system keychain.
- Authorization-code callbacks use PKCE and state validation.
- Device public keys can be used to authenticate device-originated sync requests.
- Device revocation is checked before accepting cloud mutations.
- Enterprise authorization is evaluated before memory retrieval or graph expansion.
- Authentication audit data is separated from memory audit data.

## 10. Future enterprise extensions

The control plane should leave room for:

- OIDC/SAML enterprise SSO
- SCIM provisioning/deprovisioning
- organization-enforced MFA
- managed devices
- organization-owned encryption keys
- regional identity deployment
- dedicated enterprise authentication domains
- administrator device/session review

These should be added without changing the local ORBIT memory protocol.
