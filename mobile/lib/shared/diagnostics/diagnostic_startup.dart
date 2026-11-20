import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';
import 'package:sentry_flutter/sentry_flutter.dart';
import 'package:uuid/uuid.dart';

import '../http/app_http_client.dart';
import 'diagnostic_filter.dart';
import 'diagnostic_reporter.dart';
import 'sentry_diagnostic_reporter.dart';

/// The build's reporting destination, a Sentry DSN supplied at build time.
/// Builds made from this repository alone have none and report nothing.
const reportingDestination = String.fromEnvironment('BUZZ_SENTRY_DSN');

/// Runs [app] with the reporter this build calls for.
///
/// Without a [destination] the SDK is never initialized: a reporting library
/// given an empty destination can still install its hooks (ADR 0007). With
/// one, the SDK's own handlers capture native crashes, Flutter framework
/// errors and uncaught asynchronous Dart errors from then on. A destination
/// the SDK cannot parse is treated as none, so a configuration mistake never
/// stops the app from starting.
Future<void> runWithDiagnostics({
  required String destination,
  required FutureOr<void> Function(DiagnosticReporter reporter) app,
}) async {
  if (!_isUsable(destination)) return app(const NoopDiagnosticReporter());
  final installId = await diagnosticInstallId();
  await SentryFlutter.init(
    (options) => configureDiagnostics(options, destination),
    appRunner: () {
      // A scope tag also reaches the native SDKs, so native crashes and
      // hangs carry it too. Counts carry scope attributes, not tags.
      if (installId != null) {
        Sentry.configureScope(
          (scope) => scope
            ..setTag(DiagnosticFilter.installIdTag, installId)
            ..setAttributes({
              DiagnosticFilter.installIdTag: SentryAttribute.string(installId),
            }),
        );
      }
      final reporter = SentryDiagnosticReporter();
      // The shared HTTP client is made outside Riverpod; it counts to the same
      // reporter, so its counts follow the member like everything else.
      appHttpDiagnostics = reporter;
      return app(reporter);
    },
  );
}

const _installIdFile = 'diagnostics_install_id';

/// A random identifier for this installation, so reports from one device can
/// be matched up across sign-ins and sign-outs. Created on first use and kept
/// in a file that device backups and transfers leave behind, so a phone
/// restored from another gets its own: on Android in the files directory,
/// excluded by the backup rules, and on iOS in an Application Support folder
/// the app marks as excluded from backup. Null if storage fails, since
/// reporting must never stop the app from starting.
Future<String?> diagnosticInstallId({
  Future<Directory> Function()? directory,
}) async {
  try {
    final dir = await (directory ?? _installIdDirectory)();
    final file = File('${dir.path}/$_installIdFile');
    if (await file.exists()) {
      // A failed read goes to the catch below and keeps the file: replacing it
      // after a passing error would split this device's reports.
      final bytes = await file.readAsBytes();
      try {
        final saved = utf8.decode(bytes).trim();
        if (saved.isNotEmpty) return saved;
      } on FormatException {
        // Not text: a damaged file is replaced below, not a reason to send
        // reports without an install ID.
      }
    }
    final created = const Uuid().v4();
    await dir.create(recursive: true);
    await file.writeAsString(created, flush: true);
    return created;
  } on Object catch (error) {
    debugPrint('[Diagnostics] no install ID: ${error.runtimeType}');
    return null;
  }
}

Future<Directory> _installIdDirectory() async {
  if (!Platform.isIOS) return getApplicationSupportDirectory();
  final path = await const MethodChannel(
    'buzz/diagnostics',
  ).invokeMethod<String>('installIdDirectory');
  if (path == null) throw StateError('no install ID directory');
  return Directory(path);
}

bool _isUsable(String destination) {
  if (destination.isEmpty) return false;
  try {
    final dsn = Dsn.parse(destination);
    // The native SDKs also need an HTTP(S) host and a public key, which the
    // Dart parser lets through; they would fail after Dart installs its
    // handlers. The scheme is checked on the raw text because Dart lowercases
    // it and Cocoa does not.
    if (!(destination.startsWith('https://') ||
            destination.startsWith('http://')) ||
        (dsn.uri?.host ?? '').isEmpty ||
        dsn.publicKey.isEmpty ||
        dsn.projectId.isEmpty) {
      throw const FormatException();
    }
    return true;
  } on Object catch (error) {
    // Dsn.parse throws ArgumentError or FormatException; the message would
    // repeat the destination, so only the type is printed.
    debugPrint(
      '[Diagnostics] reporting off: unusable destination '
      '(${error.runtimeType})',
    );
    return false;
  }
}

/// Applies the reporting settings from `docs/mobile-diagnostic-reporting.md`.
void configureDiagnostics(SentryFlutterOptions options, String destination) {
  options
    ..dsn = destination
    ..sendDefaultPii = false
    ..attachScreenshot = false
    ..attachViewHierarchy = false
    // Breadcrumbs come only from the app's fixed list and the SDK's
    // foreground/background entries; native ones can carry addresses.
    ..enableAutoNativeBreadcrumbs = false
    // On iOS this is Cocoa's own network breadcrumbs, which keep each
    // request's query and fragment and reach native crash reports, which
    // never pass through the filter below.
    ..recordHttpBreadcrumbs = false
    // Otherwise the SDK replaces debugPrint in non-debug builds with one that
    // records a breadcrumb and prints nothing.
    ..enablePrintBreadcrumbs = false
    ..enableUserInteractionBreadcrumbs = false
    ..enableAppLifecycleBreadcrumbs = true
    // Main-thread hangs, at the SDK's default thresholds. Dart runs on the
    // main thread, so blocking Dart work is what these catch.
    ..enableAppHangTracking = true
    ..anrEnabled = true
    ..enableWatchdogTerminationTracking = true
    ..beforeSend = ((event, hint) => DiagnosticFilter.filterEvent(event))
    ..beforeBreadcrumb = ((crumb, hint) =>
        DiagnosticFilter.filterBreadcrumb(crumb))
    ..beforeSendMetric = ((metric, hint) =>
        DiagnosticFilter.filterMetric(metric));
}
