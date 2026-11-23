import 'package:sentry_flutter/sentry_flutter.dart';

import 'diagnostic_reporter.dart';

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

  /// Breadcrumb category the app writes for the screen the person moved to.
  static const navigationCategory = 'navigation';

  /// The one data key of a navigation breadcrumb: a `DiagnosticScreen` name.
  static const screenKey = 'screen';

  /// Breadcrumb category of the SDK's foreground/background entries.
  static const lifecycleCategory = 'app.lifecycle';

  /// Tag carrying the random per-installation identifier on every report, and
  /// the attribute that carries it on every count.
  static const installIdTag = 'install.id';

  /// Attributes the app itself sets on counts and breadcrumbs.
  static const appAttributes = {'backend', 'endpoint', 'outcome', 'failure'};

  /// Attribute prefixes the SDK sets on every count: its own name and
  /// version, the release, the member, and the operating system and device.
  static const _sdkAttributePrefixes = ['sentry.', 'user.', 'os.', 'device.'];

  /// Fixed wording that replaces an error's own text.
  static const removedText = 'Error text removed before sending.';

  static const _breadcrumbCategories = {
    networkCategory,
    navigationCategory,
    lifecycleCategory,
  };

  /// Device fields that may leave: the hardware and its state, never the
  /// name a person gave it or its locale and timezone.
  static const _device = {
    'family',
    'model',
    'model_id',
    'arch',
    'battery_level',
    'orientation',
    'manufacturer',
    'brand',
    'screen_width_pixels',
    'screen_height_pixels',
    'screen_density',
    'screen_dpi',
    'online',
    'charging',
    'low_memory',
    'simulator',
    'memory_size',
    'free_memory',
    'usable_memory',
    'storage_size',
    'free_storage',
    'external_storage_size',
    'external_free_storage',
    'boot_time',
    'processor_count',
    'cpu_description',
    'processor_frequency',
    'device_type',
    'battery_status',
  };

  /// App fields that may leave: which build, and its memory and state.
  static const _app = {
    'app_name',
    'app_version',
    'app_identifier',
    'app_build',
    'build_type',
    'app_start_time',
    'app_memory',
    'in_foreground',
  };

  static const _os = {
    'name',
    'version',
    'build',
    'kernel_version',
    'rooted',
    'raw_description',
  };

  static const _runtime = {
    'name',
    'compiler',
    'version',
    'raw_description',
    'build',
  };

  static Map<String, dynamic> _only(
    Map<String, dynamic> json,
    Set<String> keys,
  ) => {
    for (final entry in json.entries)
      if (keys.contains(entry.key)) entry.key: entry.value,
  };

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
      appAttributes.contains(key) ||
      key == installIdTag ||
      _sdkAttributePrefixes.any(key.startsWith);

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
    // Within those, only listed fields leave. The native SDKs' contexts carry
    // more, such as the device's locale and timezone, which the Dart SDK keeps
    // and writes back out.
    final contexts = event.contexts;
    if (contexts.device case final device?) {
      contexts.device = SentryDevice.fromJson(_only(device.toJson(), _device));
    }
    if (contexts.app case final app?) {
      contexts.app = SentryApp.fromJson(_only(app.toJson(), _app));
    }
    if (contexts.operatingSystem case final os?) {
      contexts.operatingSystem = SentryOperatingSystem.fromJson(
        _only(os.toJson(), _os),
      );
    }
    contexts.runtimes = [
      for (final runtime in contexts.runtimes)
        SentryRuntime.fromJson(_only(runtime.toJson(), _runtime)),
    ];
    event.tags?.removeWhere((key, _) => !_allowsTag(key));
    // Nothing sets additional data today; drop it so nothing later can.
    // ignore: deprecated_member_use
    event.extra = null;
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
    if (crumb.category == navigationCategory) return _screenCrumb(crumb);
    crumb.message = switch (crumb.message) {
      final message? => stripUrlTails(message),
      null => null,
    };
    if (crumb.category == networkCategory) {
      crumb.data?.removeWhere((key, _) => !appAttributes.contains(key));
    }
    return crumb;
  }

  /// A navigation breadcrumb leaves as its screen name and nothing else, and
  /// only when that name is on the app's list.
  static Breadcrumb? _screenCrumb(Breadcrumb crumb) {
    final screen = crumb.data?[screenKey];
    if (!DiagnosticScreen.values.any((value) => value.name == screen)) {
      return null;
    }
    crumb.message = null;
    crumb.data = {screenKey: screen};
    return crumb;
  }

  /// Filters a count.
  static SentryMetric filterMetric(SentryMetric metric) {
    metric.attributes.removeWhere((key, _) => !allowsAttribute(key));
    return metric;
  }
}
