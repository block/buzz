# buzz-push-gateway

Push notification gateway. Bridges Nostr events to push notification services (APNs for iOS, FCM for Android).

**Flow:** Event arrives at relay → relay notifies push gateway → gateway formats push payload → sends to APNs/FCM for offline or backgrounded clients.

**Related:**
- [MobileClient](mobile-client)
- [DesktopClient](desktop-client)
