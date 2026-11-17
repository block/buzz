# Mobile diagnostic reporting

Status: step one agreed, not yet implemented.

Terminology is defined in [CONTEXT.md](../CONTEXT.md#mobile-builds). The decisions behind this spec are in [ADR 0007](adr/0007-diagnostic-reporting-follows-the-destination.md) and [ADR 0008](adr/0008-sentry-for-mobile-diagnostic-reporting.md).

## Problem statement

When the mobile app crashes or misbehaves, nobody who can fix it finds out. Uncaught errors are dropped, failures are caught and shown as a generic message or nothing, and there is no record of how often the app's connections to its backends fail. A member who reports "it was slow" or "it crashed" cannot be helped without their phone in hand, and a backend problem that affects many members at once is invisible from the client side.

## Solution

A build that is given a [reporting destination](../CONTEXT.md#reporting-destination) does [diagnostic reporting](../CONTEXT.md#diagnostic-reporting): it sends crashes, unexpected errors and networking measurements to that destination, tagged with the member who hit them. A build without a reporting destination sends nothing and behaves exactly as today. Reports never contain what people write or the names of the places they write it.

## User stories

1. As a developer, I want every crash of the app reported with a readable stack trace, so that I can fix crashes I never saw myself.
2. As a developer, I want every uncaught Dart and Flutter error reported, so that errors the app currently drops become visible.
3. As a developer, I want a report when the relay sends something the app cannot decode, so that protocol mismatches are found quickly.
4. As a developer, I want a report when the relay rejects something the app sent as invalid, so that client bugs in what we send are found.
5. As a developer, I want a report when a Builderlab or push gateway response cannot be understood, so that contract drift is caught.
6. As a developer, I want a report when data saved on the device cannot be read back, so that storage corruption and migration bugs surface.
7. As a developer, I want each report to carry the trail of recent networking events, so that I can see what the connection was doing before the fault.
8. As a developer, I want each report to carry the member's public key, so that I can follow up with the person who hit it.
9. As a developer, I want timeouts, disconnects, being offline, rate limits and refused sign-ins counted and not reported as errors, so that the error list contains only bugs someone can fix.
10. As a developer, I want every network request and relay connection event counted with its backend, endpoint, outcome and kind of failure, so that I can see which backend and which operation is failing.
11. As a developer, I want request durations measured per backend and endpoint, so that slowness shows up as well as failure.
12. As a developer, I want those counts to carry the member, so that I can answer "who hit this failure, and when" without a separate log.
13. As a person on call, I want to be alerted when the failure rate for a backend or endpoint spikes, so that an outage seen only by clients is noticed.
14. As a distributor of an [internal build](../CONTEXT.md#internal-build), I want to turn reporting on by supplying one build-time setting, so that my organization's reports go to my own destination.
15. As a distributor, I want stack traces from my release builds to be readable, so that reports are usable.
16. As someone building the app from source, I want a build with no reporting destination to send nothing and start no reporting code, so that the open-source app reports to no one by default.
17. As a member, I want reports to never include message content, channel or community names, private keys or other secrets, so that diagnosing faults does not expose what I write.
18. As a member, I want reporting to never slow the app or block what I am doing, so that diagnosis does not cost me anything.
19. As a developer, I want signing out or switching accounts to change who later reports are attributed to, so that reports are never tagged with the wrong member.
20. As a reviewer, I want all reporting to go through one small interface, so that I can check what leaves the device in one place.

## Implementation decisions

- **One reporting module.** All app code reports through a single interface with three operations: report an error, count a networking event (with an optional duration), and leave a breadcrumb. It has two implementations: one backed by the Sentry Flutter SDK and one that does nothing. Nothing outside this module imports the SDK.
- **The reporting destination is a build-time setting**, named with the implementation to match the existing `BUZZ_` build settings. When it is absent, the app uses the do-nothing implementation and never initializes the SDK. There is no check on the kind of build, no in-app switch and no notice ([ADR 0007](adr/0007-diagnostic-reporting-follows-the-destination.md)).
- **Crashes and uncaught errors** are captured by the SDK's own handlers for native crashes, Flutter framework errors and uncaught asynchronous Dart errors, installed at startup around the app.
- **Explicit error reports** are added at the places where the app meets something that should never happen: a relay frame or query response that cannot be decoded, a relay rejection whose reason says the app sent something invalid (except a refusal the app already handles as an expected outcome, such as an archived channel or a channel name that is taken, which is counted), an account or push gateway response that cannot be understood, saved data that cannot be read back, and an event subscriber that throws while handling an event.
- **Routine failures are counted, never reported.** Timeouts, disconnects, unreachable or insecure hosts, rate limits, refused or expired sign-ins and server errors increment a count. That includes the relay's own errors, such as a query that timed out or too many subscriptions. Of the reasons a relay gives for a rejection, only the one that marks the request as invalid is reported, with the exception above; every other reason is counted.
- **Networking measurements.** Every request through the app's shared HTTP client and every relay connection event (connected, disconnected, reconnect attempt, request timed out, subscription closed by the relay) is counted with four attributes: backend, endpoint, outcome and kind of failure. The backend is the host name of the server the app was talking to. Endpoints are names from a fixed list, never raw paths. Durations are recorded for requests and for time to connect. HTTP is measured where the app's shared HTTP client already classifies transport failures; relay connections are measured by hand in the relay session, since the SDK documents no WebSocket instrumentation.
- **Counts are handed to the SDK as they happen**, including while the network is down. The app does no buffering of its own, and the SDK's default limit on unsent data is kept. Accepted consequence: in a long outage the oldest unsent reports can be dropped.
- **No log lines in step one.** The counts carry the member and the time. The first implementation task confirms that the destination can filter counts by member; if it cannot, the fallback is one log line per outage, sent after it ends.
- **Breadcrumbs come from a fixed list** written by the app: networking events and the app moving to the foreground or background. The SDK's automatic breadcrumbs that could carry content or addresses are turned off or filtered.
- **Who a report is about.** The member's public key is set as the report's user when a session starts and cleared when it ends. When the session is a Builderlab account, the Builderlab's origin and the account's subject identifier are added, never its email address.
- **One filter before anything leaves.** Errors, breadcrumbs and counts all pass through a single filter that removes URL query strings and fragments, drops any attribute not on the allowed list, and replaces free-text failure messages with fixed wording. The SDK's options for sending personal data, screenshots, view hierarchies and session recordings stay off.
- **Readable stack traces** need the build's debug symbols at the destination. Uploading them belongs to the release process of whoever distributes the build; this repository documents the requirement and adds nothing organization-specific.
- **Both platforms.** Step one covers the main app on iOS and Android. Background work that runs the app's Dart code in its own engine, as decided for Android push handling in [ADR 0006](adr/0006-android-notifications-run-app-dart-code.md), starts the reporting module the same way once it exists.
- **Reporting never blocks the app.** A call into the reporting module returns at once and nothing waits on the destination. A failure inside reporting is dropped and never reaches the code that made the call.

## Testing decisions

- A good test here asserts what would be reported for something the user or the network does, never how the reporting module is called internally.
- Tests replace the reporting interface with a fake that records what it was given. This is the only new seam.
- Journey tests extend the existing relay and account journeys with steps such as: a malformed relay frame produces exactly one error report; a timeout or disconnect is counted with the right backend, endpoint and failure kind and produces no report; signing out changes who later reports are about.
- One table-driven test covers the filter: each row is an input (a URL with a query string, an attribute not on the list, a free-text relay message) and what must come out.
- One test checks that a build with no reporting destination never starts the SDK.
- One test checks that a reporting call that fails does not fail or delay the operation that made it.
- The SDK itself is not tested. Prior art: the existing widget tests that override boundary providers for the relay, HTTP and storage.

## Out of scope

- The iOS notification service extension and the iOS share extension.
- Log lines, traces and spans, and joining mobile reports to relay traces.
- Holding counts back in the app while the network is down. The SDK's own store of unsent data is the only buffer.
- Requests that bypass the shared HTTP client, such as those made by native media players and image loaders.
- Alert rules, dashboards and the destination's own configuration, which belong to whoever owns the destination.
- Diagnostic reporting in a [public build](../CONTEXT.md#public-build). Nothing prevents it, but what a report carries was chosen for internal builds. Turning it on there first needs the member's public key and the server host names removed, the rest of the list reviewed, and a way to tell people ([ADR 0007](adr/0007-diagnostic-reporting-follows-the-destination.md)). Because identity and every attribute pass through one module, that is a small code change.
- Usage or behavior data of any kind.
- Fixing the bugs the reports will reveal, including two found while planning: one subscriber that throws can lose the rest of a batch of events, and a catch-all in account requests presents every error as an outage.

## Further notes

- Depends on the iOS move to Swift Package Manager, since the current Sentry SDK installs on iOS only that way ([ADR 0008](adr/0008-sentry-for-mobile-diagnostic-reporting.md)).
- Not yet verified: whether the destination can filter counts by member, and how Android's store of unsent data behaves.
- A relay's host name can identify a community. It is reported anyway so that two relays can be told apart; an internal build talks only to its own organization's relay, so the name tells the destination's owner nothing new.
