# Sender information in iOS notifications

Buzz must show a sender's name and avatar even when the main app has never
loaded that sender's profile. The notification service extension fetches and
caches the information it needs. The main app supplies signed-in accounts,
relay configuration and credentials in the shared Keychain. It does not export
profiles, channel details or avatar images for notifications.

## Display behavior

The extension uses one freshness threshold of one hour for profiles, channel
details, and avatar refresh checks. The period starts at the last successful
check. Reading a record does not extend it. When a record is older, the
extension tries to refresh it and can use the previous record if the refresh
fails. This setting does not extend authorization. Image requests also follow
HTTP caching rules, which can avoid downloading unchanged image data again.

After it resolves the message, the extension allows up to 1.5 seconds for the
sender and channel details. It then delivers the best available notification.
If no sender name or avatar is available, it omits those values where iOS
supports this. It must not use a public key as the displayed name.

When a sender changes their avatar, the previous image remains usable until
the replacement arrives. When a verified profile removes the avatar, the old
image is no longer shown. A late response must not restore a removed avatar.

The extension supports the same avatar sources as the app: Buzz uploads,
external HTTP(S) images and supported inline images. It uses existing server
thumbnails for eligible Buzz uploads. A separate resized-image cache is not
required. Animated avatars use the sender-selected still image. Inline SVGs
are rendered natively. Image decoding has size limits; SVG decoding also limits
nesting and repeated references. Embedded fonts, external SVG resources and
recursive references are rejected. An image that cannot be decoded follows the
same stale-image or missing-image behavior as a failed download.
An empty profile query is not a verified removal and does not clear stale data.

## Fetching and storage

The extension uses native networking and its own persistent storage. Image
responses use URLSession and URLCache; verified profiles and channel details
have a separate bounded store. Each account and relay has separate cache data.
Credentials are not stored in these caches. Profile and channel requests run
independently, so a large or slow channel roster cannot block a sender profile
or avatar that is available before the delivery deadline.
When only channel metadata or membership refreshes successfully, the extension
keeps that record and removes its old counterpart in the same write. It does
not derive a group identity or recipient count from a mixed pair. If neither
request succeeds, the previous pair remains available.

Delivery happens only once. Ordinary requests can continue after delivery if
iOS keeps the process alive. A late profile response can start an avatar fetch,
and completed results can be saved for the next notification. These requests
use normal network timeouts. There are no background sessions, and successful
notification delivery does not depend on work continuing after delivery.

A cache miss is normal. Interrupted writes must leave complete old or new
records, not partial records. Invalid cache entries are reported and treated
as misses. The extension must work after iOS removes cache files or starts a
new process.

Removing an account immediately prevents access to its cached information.
The extension deletes the account's cache files when it next runs. Results
from requests started before removal cannot become usable in a new sign-in.

## Validation

Tests cover a sender never loaded by the app, cache hits and misses, stale
records, avatar replacement and removal, account removal, late responses and
exactly-once delivery. Storage tests cover concurrent access and interruption.
Device validation checks actual notification appearance, authenticated image
cache reuse, process restarts and notification bursts. In particular, accepting
nil name and image values in an API does not by itself prove how iOS renders
those values.

Persistent image caching for the main app is separate from this change.
