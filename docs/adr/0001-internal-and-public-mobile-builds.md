# Internal and public mobile builds

The mobile app is built either as a [public build](../../CONTEXT.md#public-build) or as an [internal build](../../CONTEXT.md#internal-build), chosen by an explicit build-time setting, `BUZZ_DISTRIBUTION=internal|public`. Each build trusts exactly one Builderlab, named by `BUZZ_BUILDERLAB_URL`, and sends that Builderlab's relay assertions only to the relays listed in `BUZZ_BUILDERLAB_RELAYS`. With none of these set, the app is a Nostr-only development build. The former `BUZZ_AUTH_DOMAINS`, `BUZZ_ORGANIZATION_URL` and `BUZZ_ORGANIZATION_RELAYS` settings stop the app at startup with a migration error rather than silently produce a Nostr-only build. A public build keeps QR pairing and multiple communities. An internal build takes exactly one relay host and reaches only its organization's community: its first screen offers a single "Sign in" button that goes to the organization's single sign-on, with no QR pairing and no community switching, and signing out removes that community from the device but keeps its key there. Communities that a previous build saved on other relays stay stored but hidden.

We chose this so that an organization's members cannot reach public communities from the organization's app, and the public app cannot reach an organization's community, while the app keeps a single Builderlab session. The public build needs no knowledge of any organization: an organization's relay already rejects clients that cannot present its Builderlab's assertion, so the public build relies on that rejection instead of a list of organization hosts.

## Considered options

- **One build that supports both Builderlabs at once.** This was the previous design. It needed separate account and organization settings and two concurrent sessions, and it let an organization's app sign in to the public Builderlab from its first screen.
- **Inferring the build kind from the Builderlab URL.** Rejected because it requires a table of known Builderlab hosts in code and breaks for any organization not in that table. The explicit `BUZZ_DISTRIBUTION` setting lets any organization ship its own internal build against its own Builderlab.

## Consequences

Upgrading an internal build that had joined public communities or signed in to the public Builderlab clears that public session and hides those communities. Their saved keys stay on the device.

An internal build has no QR pairing, so it cannot receive a member's key from another device. When the organization's Builderlab already binds an identity to the member, Sign in uses that key from this device or restores it from the Builderlab's key backup. An identity the Builderlab never backed up cannot be restored, and Sign in says so rather than creating a new identity.

On iOS, the native sign-in browser currently opens only Builderlab's own hosts, so an internal build's `BUZZ_BUILDERLAB_URL` must be a Builderlab tenant until that allowlist follows build configuration.
