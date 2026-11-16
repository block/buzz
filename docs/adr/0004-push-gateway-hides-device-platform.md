# The push gateway hides each installation's platform from the relay

A relay should learn as little as possible about the devices its members use, so that device details can't help correlate one person's identities. So a [push gateway](../../CONTEXT.md#push-gateway) serves one mobile app (an internal build, the public build or a fork's app) on every platform it ships on, rather than one gateway per platform. The installation's platform and push service live only inside the gateway-sealed grant, which the relay stores but can't read. The relay's push lease carries the same fixed `transport` value on every platform, uses one set of priority classes that the gateway maps to each platform, and follows the same renewal schedule on every platform. Every delivery request the relay sends goes to one platform-neutral gateway route and is the same whatever the device is.

## Considered options

- **One gateway per platform, with the relay choosing a gateway by platform.** It would keep the gateway's one-transport data model, but the relay would have to know each installation's platform.
- **Remove `transport` from the lease.** It would tidy the format, but released clients and relays require it, so removing it would need an ordered rollout across clients and relays. Since every platform sends the same constant, it tells the relay nothing, and the leftover value (which names APNs) is the accepted cost.

## Consequences

The relay still sees each installation's IP address and connection behavior, which correlate more strongly than platform does. This decision removes the one signal that the push protocol itself added.

One installation still holds a single grant per relay, so two identities on one phone publish leases with the same opaque endpoint, and the relay can link them. Per-identity grants are out of scope for now, but we want to keep them possible. A fork can run its own gateway that issues them. Our gateway allows one delegation per installation and relay today, so offering them there later means relaxing that rule.

Android installations use Firebase Cloud Messaging and require Google Play services. We are interested in also supporting UnifiedPush later, for Android phones without Google Play services (for example privacy-focused Android builds), which fits Buzz's privacy and self-sovereignty goals. That would mean the gateway posting wake signals to push servers its users choose, which brings its own security work. It becomes worth doing once Buzz ships outside Google Play. This ADR doesn't commit to it; it would be one more push service the gateway knows about, with no change on the relay.
