import 'package:sentry_flutter/sentry_flutter.dart';

import '../relay/relay_call_error.dart';
import '../relay/signed_event_relay.dart' show SignedEventUnacknowledged;

/// Says which relay call an error came from and why it failed, on a report
/// whose error is a [RelayCallError], or a [SignedEventUnacknowledged] caused
/// by one.
///
/// An error that escapes uncaught reaches the SDK from the zone's handler,
/// not through the reporter, and the filter replaces its text, so without
/// this a report of one says only that some relay call failed. The reason is
/// the relay's reason prefix or a timeout, never the relay's own text.
abstract final class RelayErrorTags {
  /// Tag naming the relay call: publish, query, history or subscribe.
  static const callTag = 'relay.call';

  /// Tag naming the reason class, such as invalid or rate-limited.
  static const reasonTag = 'relay.reason';

  /// Adds the tags to [event], and splits its issue by call and reason on top
  /// of the usual grouping by stack, so a rate limit and an invalid request
  /// from the same call site are not one issue. Other events pass unchanged.
  static SentryEvent apply(SentryEvent event) {
    final thrown = event.throwable;
    final error = thrown is SignedEventUnacknowledged ? thrown.cause : thrown;
    if (error is! RelayCallError) return event;
    final call = error.call.name;
    final reason = error.reason.tag;
    event.tags = {...?event.tags, callTag: call, reasonTag: reason};
    event.fingerprint = [
      SentryEvent.defaultFingerprint,
      'relay.call:$call',
      'relay.reason:$reason',
    ];
    return event;
  }
}
