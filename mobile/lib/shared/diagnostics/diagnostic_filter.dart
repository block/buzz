import 'package:sentry_flutter/sentry_flutter.dart';

/// The one filter everything passes through before it leaves the device.
///
/// It removes URL query strings and fragments, keeps only attributes on the
/// allowed list, and drops breadcrumbs that are not from the app's fixed list.
/// See "One filter before anything leaves" in
/// `docs/mobile-diagnostic-reporting.md`.
abstract final class DiagnosticFilter {
  /// Breadcrumb category the app writes for networking events.
  static const networkCategory = 'network';

  /// Breadcrumb category of the SDK's foreground/background entries.
  static const lifecycleCategory = 'app.lifecycle';

  /// Attributes the app itself sets on counts and breadcrumbs.
  static const appAttributes = {'backend', 'endpoint', 'outcome', 'failure'};

  /// Attribute prefixes the SDK sets on every count: its own name and
  /// version, the release, the member, and the operating system and device.
  static const _sdkAttributePrefixes = ['sentry.', 'user.', 'os.', 'device.'];

  static const _breadcrumbCategories = {networkCategory, lifecycleCategory};

  static final _urlTail = RegExp(
    r'''(\b[a-z][a-z0-9+.-]*://[^\s?#'"]*)[?#][^\s'"]*''',
    caseSensitive: false,
  );

  /// Removes the query string and fragment of every URL in [text].
  static String stripUrlTails(String text) =>
      text.replaceAllMapped(_urlTail, (match) => match.group(1)!);

  /// Whether an attribute [key] may leave the device.
  static bool allowsAttribute(String key) =>
      appAttributes.contains(key) || _sdkAttributePrefixes.any(key.startsWith);

  /// The app tags a report with where it came from; the SDK tags it with
  /// its origin and environment.
  static bool _allowsTag(String key) =>
      key == 'operation' || key.startsWith('event.') || allowsAttribute(key);

  /// Filters an error report.
  static SentryEvent filterEvent(SentryEvent event) {
    event.request = null;
    event.tags?.removeWhere((key, _) => !_allowsTag(key));
    event.message = switch (event.message) {
      final message? => SentryMessage(stripUrlTails(message.formatted)),
      null => null,
    };
    for (final exception in event.exceptions ?? const <SentryException>[]) {
      exception.value = switch (exception.value) {
        final value? => _exceptionValue(exception.type, value),
        null => null,
      };
    }
    event.breadcrumbs = [
      for (final crumb in event.breadcrumbs ?? const <Breadcrumb>[])
        ?filterBreadcrumb(crumb),
    ];
    return event;
  }

  /// Filters a breadcrumb, or returns null to drop it.
  static Breadcrumb? filterBreadcrumb(Breadcrumb? crumb) {
    if (crumb == null || !_breadcrumbCategories.contains(crumb.category)) {
      return null;
    }
    crumb.message = switch (crumb.message) {
      final message? => stripUrlTails(message),
      null => null,
    };
    if (crumb.category == networkCategory) {
      crumb.data?.removeWhere((key, _) => !appAttributes.contains(key));
    }
    return crumb;
  }

  /// Filters a count.
  static SentryMetric filterMetric(SentryMetric metric) {
    metric.attributes.removeWhere((key, _) => !allowsAttribute(key));
    return metric;
  }

  /// A [FormatException]'s text quotes the input it could not parse, which
  /// can be message content, so only its fixed wording is kept.
  static String _exceptionValue(String? type, String value) =>
      type == 'FormatException'
      ? 'The input could not be parsed.'
      : stripUrlTails(value);
}
