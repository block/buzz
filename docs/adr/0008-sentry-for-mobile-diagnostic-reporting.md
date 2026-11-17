# Sentry for mobile diagnostic reporting

The mobile app sends its [diagnostic reporting](../../CONTEXT.md#diagnostic-reporting) with the Sentry Flutter SDK, so a [reporting destination](../../CONTEXT.md#reporting-destination) is a Sentry project. We chose Sentry because, of the Flutter SDKs compared in November 2026, it was the only one with crashes (Dart and native), non-fatal errors, logs and custom metrics in a single SDK, and it had by far the most frequent releases.

## Considered options

- **Datadog.** The relay exports OpenTelemetry traces and Prometheus metrics, which a deployment can collect in Datadog, so client and server data could sit together there. Its Flutter SDK documents no custom metrics from the client, and its native crash reporting is off by default.
- **Bugsnag.** Comparable crash and error coverage, but no logs, no custom metrics, and three releases of its Flutter SDK in the year before the comparison.

## Consequences

The current Sentry Flutter SDK installs on iOS only through Swift Package Manager, so the iOS app's plugins move to Swift Package Manager first.

None of the three SDKs documents any WebSocket instrumentation. Everything reported about relay connections is written by hand in the app, whichever vendor is used.

Mobile reports are not joined to the relay's traces. Correlating a mobile fault with relay behavior is done by time and member until a link between the two is built.
