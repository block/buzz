import 'package:sentry_flutter/sentry_flutter.dart';

/// The one filter every Dart report, breadcrumb and count passes through
/// before it leaves the device. Reports the native SDKs capture themselves
/// (native crashes, app hangs) do not pass through it.
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

  /// Tag carrying the random per-installation identifier on every report.
  static const installIdTag = 'install.id';

  /// Attributes the app itself sets on counts and breadcrumbs.
  static const appAttributes = {'backend', 'endpoint', 'outcome', 'failure'};

  /// Attribute prefixes the SDK sets on every count: its own name and
  /// version, the release, the member, and the operating system and device.
  static const _sdkAttributePrefixes = ['sentry.', 'user.', 'os.', 'device.'];

  /// Fixed wording that replaces an error's own text.
  static const removedText = 'Error text removed before sending.';

  static const _breadcrumbCategories = {networkCategory, lifecycleCategory};

  /// Report contexts that may leave the device: the app, operating system and
  /// device details, the Dart runtime, and the SDK's trace identifiers. The
  /// SDK adds others on its own, such as accessibility settings and locale.
  static const _contexts = {
    'app',
    'device',
    'os',
    'runtime',
    'runtimes',
    'trace',
  };

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

  /// The app tags a report with where it came from and which installation
  /// sent it; the SDK tags it with its origin and environment.
  static bool _allowsTag(String key) =>
      key == 'operation' ||
      key == installIdTag ||
      key.startsWith('event.') ||
      allowsAttribute(key);

  /// Filters an error report.
  static SentryEvent filterEvent(SentryEvent event) {
    event.request = null;
    // Kept by name, like tags: the SDK adds contexts such as the widget and
    // error description, accessibility settings and locale.
    event.contexts.removeWhere((key, _) => !_contexts.contains(key));
    // The name a person gave their device.
    event.contexts.device?.name = null;
    event.tags?.removeWhere((key, _) => !_allowsTag(key));
    event.message = switch (event.message) {
      final message? => SentryMessage(stripUrlTails(message.formatted)),
      null => null,
    };
    for (final exception in event.exceptions ?? const <SentryException>[]) {
      // An error's text can quote its input, a server's response body or a
      // URL, so only its type, mechanism type and stack trace are kept.
      if (exception.value != null) exception.value = removedText;
      exception.mechanism?.data = null;
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
}
