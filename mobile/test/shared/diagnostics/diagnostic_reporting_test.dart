import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/diagnostics/diagnostic_filter.dart';
import 'package:buzz/shared/diagnostics/diagnostic_member_binding.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/diagnostics/diagnostic_startup.dart';
import 'package:buzz/shared/diagnostics/sentry_diagnostic_reporter.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
// ignore: depend_on_referenced_packages, implementation_imports
import 'package:sentry/src/deduplication/deduplication_event_processor.dart';
import 'package:sentry_flutter/sentry_flutter.dart';
// The SDK's own processors and integrations, so tests see what it really
// sends rather than hand-built events.
// ignore: implementation_imports
import 'package:sentry_flutter/src/breadcrumbs/debug_print_integration.dart';
// ignore: implementation_imports
import 'package:sentry_flutter/src/exception/flutter_error_integration.dart';
// ignore: implementation_imports
import 'package:sentry_flutter/src/enrichment/flutter_enricher_event_processor.dart';
// ignore: implementation_imports
import 'package:sentry_flutter/src/enrichment/widget_event_processor.dart';
// ignore: implementation_imports
import 'package:sentry_flutter/src/url_filter/url_filter_event_processor.dart';
// ignore: implementation_imports
import 'package:sentry_flutter/src/exception/flutter_exception_event_processor.dart';
// ignore: implementation_imports
import 'package:sentry_flutter/src/exception/platform_exception_event_processor.dart';

void main() {
  group('what leaves the device', () {
    final options = SentryFlutterOptions();
    configureDiagnostics(options, 'https://key@example.org/1');

    Breadcrumb? crumb(Breadcrumb input) =>
        options.beforeBreadcrumb!(input, Hint());

    Future<SentryEvent> event(SentryEvent input) async =>
        (await options.beforeSend!(input, Hint()))!;

    Future<Map<String, SentryAttribute>> metric(
      Map<String, SentryAttribute> attributes,
    ) async => (await options.beforeSendMetric!(
      SentryCounterMetric(
        timestamp: DateTime(2026),
        name: 'network.event',
        value: 1,
        traceId: SentryId.newId(),
        attributes: attributes,
      ),
      Hint(),
    ))!.attributes;

    final rows = <(String, Future<Object?> Function(), Object?)>[
      (
        'an HTTP breadcrumb from the SDK is dropped',
        () async => crumb(
          Breadcrumb.http(url: Uri.parse('https://e.org'), method: 'GET'),
        ),
        null,
      ),
      (
        'a navigation breadcrumb is dropped',
        () async => crumb(Breadcrumb(category: 'navigation', message: 'Chat')),
        null,
      ),
      (
        'a lifecycle breadcrumb is kept',
        () async => crumb(
          Breadcrumb(category: 'app.lifecycle', data: {'state': 'paused'}),
        )?.data,
        {'state': 'paused'},
      ),
      (
        'a network breadcrumb keeps only its attributes',
        () async => crumb(
          Breadcrumb(
            category: 'network',
            data: {'backend': 'relay.example.org', 'channel': 'secret-room'},
          ),
        )?.data,
        {'backend': 'relay.example.org'},
      ),
      (
        'a URL in a message loses its query string and fragment',
        () async => (await event(
          SentryEvent(
            message: SentryMessage(
              'failed https://e.org/p?token=abc#frag and wss://r.org/x?y=1',
            ),
          ),
        )).message?.formatted,
        'failed https://e.org/p and wss://r.org/x',
      ),
      (
        'an event without a message gets none',
        () async => (await event(SentryEvent())).message,
        null,
      ),
      (
        'additional data is dropped',
        () async => (await event(
          // ignore: deprecated_member_use
          SentryEvent(extra: {'body': 'private-marker'}),
        )).extra, // ignore: deprecated_member_use
        null,
      ),
      (
        'a tag not on the list is dropped',
        () async => (await event(
          SentryEvent(
            tags: {
              'operation': 'relayFrame',
              'install.id': 'i',
              'channel': 'x',
            },
          ),
        )).tags,
        {'operation': 'relayFrame', 'install.id': 'i'},
      ),
      (
        'a count keeps its attributes and the member, nothing else',
        () async => (await metric({
          'backend': SentryAttribute.string('relay.example.org'),
          'user.id': SentryAttribute.string('ab'),
          'path': SentryAttribute.string('/channels/secret'),
        })).keys.toSet(),
        {'backend', 'user.id'},
      ),
    ];

    for (final (name, input, expected) in rows) {
      test(name, () async => expect(await input(), expected));
    }

    test('personal data, screenshots and native breadcrumbs stay off', () {
      expect(options.sendDefaultPii, isFalse);
      expect(options.attachScreenshot, isFalse);
      expect(options.attachViewHierarchy, isFalse);
      expect(options.enableAutoNativeBreadcrumbs, isFalse);
      expect(options.recordHttpBreadcrumbs, isFalse);
      expect(options.replay.sessionSampleRate, isNull);
      expect(options.tracesSampleRate, isNull);
    });

    test('main-thread hang detection is on at the default thresholds', () {
      final defaults = SentryFlutterOptions();
      expect(options.enableAppHangTracking, isTrue);
      expect(options.anrEnabled, isTrue);
      expect(options.enableWatchdogTerminationTracking, isTrue);
      expect(options.appHangTimeoutInterval, defaults.appHangTimeoutInterval);
      expect(options.anrTimeoutInterval, defaults.anrTimeoutInterval);
    });
  });

  group('a sent error', () {
    late _MemoryTransport transport;
    late SentryFlutterOptions options;
    late Hub hub;

    setUp(() {
      transport = _MemoryTransport();
      options = SentryFlutterOptions();
      configureDiagnostics(options, 'https://key@example.org/1');
      // The processors Sentry.init and SentryFlutter.init install.
      options
        ..transport = transport
        ..sendClientReports = false
        ..addEventProcessor(DeduplicationEventProcessor(options))
        ..addEventProcessor(FlutterExceptionEventProcessor())
        ..addEventProcessor(FlutterEnricherEventProcessor(options))
        ..addEventProcessor(WidgetEventProcessor())
        ..addEventProcessor(UrlFilterEventProcessor(options))
        ..addEventProcessor(PlatformExceptionEventProcessor());
      hub = Hub(options);
    });
    tearDown(() => hub.close());

    final rows = <(String, Object, String)>[
      ('a parse failure', const FormatException('private-marker'), 'Format'),
      ('a bad argument', ArgumentError.value('private-marker'), 'Argument'),
      ('a server response', StateError('body: private-marker'), 'StateError'),
      (
        'a platform failure',
        PlatformException(
          code: 'failed',
          message: 'open https://e.org/f?token=private-marker',
        ),
        'PlatformException',
      ),
    ];
    for (final (name, error, type) in rows) {
      test('keeps only the type and stack of $name', () async {
        await hub.captureException(error, stackTrace: StackTrace.current);
        final sent = transport.events.single;

        expect(jsonEncode(sent), isNot(contains('private-marker')));
        final exception = (sent['exception']['values'] as List).last as Map;
        expect(exception['type'], contains(type));
        expect(exception['value'], DiagnosticFilter.removedText);
        expect(exception['stacktrace'], isNotNull);
      });
    }

    test('a report right after a member change names the new member', () async {
      SentryDiagnosticReporter(hub: hub)
        ..setMember('a' * 64)
        ..reportError(
          StateError('failed'),
          StackTrace.current,
          operation: DiagnosticOperation.relayFrame,
        );
      await transport.first();

      expect(transport.events.single['user']['id'], 'a' * 64);
    });

    test('a test report is a test error from the test operation', () async {
      SentryDiagnosticReporter(hub: hub).sendTestReport();
      await transport.first();

      final sent = transport.events.single;
      final exception = (sent['exception']['values'] as List).last as Map;
      expect(exception['type'], 'DiagnosticTestError');
      expect(exception['stacktrace'], isNotNull);
      expect(sent['tags']['operation'], 'testReport');
    });

    test('every test report is sent, not only the first', () async {
      SentryDiagnosticReporter(hub: hub)
        ..sendTestReport()
        ..sendTestReport();
      await transport.count(2);

      expect(transport.events, hasLength(2));
    });

    testWidgets('sends only the allowed contexts', (tester) async {
      tester.platformDispatcher.accessibilityFeaturesTestValue =
          const FakeAccessibilityFeatures(accessibleNavigation: true);
      addTearDown(
        tester.platformDispatcher.clearAccessibilityFeaturesTestValue,
      );

      await tester.runAsync(
        () => hub.captureException(
          StateError('failed'),
          stackTrace: StackTrace.current,
        ),
      );

      expect(
        (transport.events.single['contexts'] as Map).keys,
        everyElement(
          isIn(['app', 'device', 'os', 'runtime', 'runtimes', 'trace']),
        ),
      );
    });

    test('keeps only listed fields within contexts', () {
      // As the native SDKs report them: the Dart SDK keeps keys it has no
      // field for and writes them back out.
      final event = SentryEvent(
        contexts: Contexts(
          device: SentryDevice.fromJson({
            'name': 'private-marker',
            'model': 'iPhone',
            'locale': 'private-marker',
            'timezone': 'private-marker',
          }),
          app: SentryApp.fromJson({
            'app_name': 'Buzz',
            'device_app_hash': 'private-marker',
            'x': 'private-marker',
          }),
          operatingSystem: SentryOperatingSystem.fromJson({
            'name': 'iOS',
            'x': 'private-marker',
          }),
          runtimes: [
            SentryRuntime.fromJson({'name': 'Dart', 'x': 'private-marker'}),
          ],
        ),
      );

      final contexts = DiagnosticFilter.filterEvent(event).contexts;

      expect(jsonEncode(contexts.toJson()), isNot(contains('private-marker')));
      expect(contexts.device!.model, 'iPhone');
      expect(contexts.app!.name, 'Buzz');
      expect(contexts.operatingSystem!.name, 'iOS');
    });

    testWidgets('leaves out what a framework error says about the screen', (
      tester,
    ) async {
      final integration = FlutterErrorIntegration();
      integration(hub, options);
      addTearDown(integration.close);

      // A Text without Directionality is a real framework build error.
      await tester.pumpWidget(const Text('private-marker'));
      expect(tester.takeException(), isA<FlutterError>());
      await tester.runAsync(transport.first);

      expect(
        jsonEncode(transport.events.single),
        isNot(contains('private-marker')),
      );
    });
  });

  test('device logs keep printing in a build that reports', () async {
    final options = SentryFlutterOptions(checker: _ReleaseChecker());
    configureDiagnostics(options, 'https://key@example.org/1');
    final printed = <String?>[];
    final original = debugPrint;
    debugPrint = (message, {wrapWidth}) => printed.add(message);
    final integration = DebugPrintIntegration();
    try {
      integration(Hub(options), options);
      debugPrint('an existing log line');
    } finally {
      integration.close();
      debugPrint = original;
    }
    expect(printed, ['an existing log line']);
  });

  for (final (name, destination) in [
    ('no reporting destination', ''),
    ('a destination the SDK cannot parse', 'https://key@example.org'),
    ('a destination without a public key', 'https://example.org/1'),
    ('a destination without an HTTP host', 'ftp://key@example.org/1'),
    ('a destination with an uppercase scheme', 'HTTPS://key@example.org/1'),
  ]) {
    test('a build with $name starts without the SDK', () async {
      DiagnosticReporter? started;
      await runWithDiagnostics(
        destination: destination,
        app: (r) => started = r,
      );

      expect(started, isA<NoopDiagnosticReporter>());
      expect(Sentry.isEnabled, isFalse);
    });
  }

  test('an installation keeps one install ID across launches', () async {
    final installation = await Directory.systemTemp.createTemp('install');
    addTearDown(() => installation.delete(recursive: true));
    Future<Directory> storage() async => installation;

    final first = await diagnosticInstallId(directory: storage);
    expect(first, isNotNull);
    expect(await diagnosticInstallId(directory: storage), first);

    final other = await Directory.systemTemp.createTemp('install');
    addTearDown(() => other.delete(recursive: true));
    expect(
      await diagnosticInstallId(directory: () async => other),
      isNot(first),
    );
  });

  test('a reporting call that fails does not reach the caller', () async {
    for (final reporter in [
      SentryDiagnosticReporter(hub: _FailingHub(), metrics: _FailingMetrics()),
      SentryDiagnosticReporter(
        hub: _FailingHub(async: true),
        metrics: _FailingMetrics(),
      ),
    ]) {
      const event = NetworkEvent(
        backend: 'relay.example.org',
        endpoint: NetworkEndpoint.relayConnection,
        outcome: NetworkOutcome.failure,
        failure: NetworkFailure.timeout,
        duration: Duration(seconds: 1),
      );
      reporter
        ..reportError(
          StateError('x'),
          null,
          operation: DiagnosticOperation.relayFrame,
        )
        ..countNetworkEvent(event)
        ..leaveBreadcrumb(const DiagnosticBreadcrumb(event))
        ..setMember('ab');
    }
    // An unhandled asynchronous failure would fail this test.
    await pumpEventQueue();
  });

  test('signing in, switching and signing out change the member', () async {
    final reporter = _RecordingReporter();
    final container = ProviderContainer(
      overrides: [
        diagnosticReporterProvider.overrideWithValue(reporter),
        activeCommunityProvider.overrideWith((ref) async => null),
      ],
    );
    addTearDown(container.dispose);
    container.listen(diagnosticMemberProvider, (_, _) {});
    final relay = container.read(relayConfigProvider.notifier);

    relay.update(baseUrl: 'https://relay.example.org', nsec: _nsec('1'));
    container.read(diagnosticMemberProvider);
    relay.update(baseUrl: 'https://relay.example.org', nsec: _nsec('2'));
    container.read(diagnosticMemberProvider);
    relay.update(baseUrl: 'https://relay.example.org');
    container.read(diagnosticMemberProvider);

    expect(reporter.members, [
      null,
      pubkeyFromNsec(_nsec('1')),
      pubkeyFromNsec(_nsec('2')),
      null,
    ]);
    expect(reporter.members[1], isNot(reporter.members[2]));
  });
}

String _nsec(String digit) =>
    nostr.Nip19.encode(prefix: nostr.Nip19Prefix.nsec, data: digit * 64);

class _RecordingReporter extends NoopDiagnosticReporter {
  final members = <String?>[];

  @override
  void setMember(String? pubkey) => members.add(pubkey);
}

class _FailingHub extends Fake implements Hub {
  _FailingHub({this.async = false});

  final bool async;

  Future<Never> _fail() =>
      async ? Future.error(StateError('down')) : throw StateError('down');

  @override
  Future<SentryId> captureException(
    dynamic throwable, {
    dynamic stackTrace,
    Hint? hint,
    SentryMessage? message,
    ScopeCallback? withScope,
  }) => _fail();

  @override
  Future<void> addBreadcrumb(Breadcrumb crumb, {Hint? hint}) => _fail();

  @override
  Future<void> configureScope(ScopeCallback callback) => _fail();
}

class _FailingMetrics extends Fake implements SentryMetrics {
  @override
  void count(
    String name,
    int value, {
    Map<String, SentryAttribute>? attributes,
  }) => throw StateError('down');
}

class _ReleaseChecker extends RuntimeChecker {
  @override
  bool isDebugMode() => false;
  @override
  bool isReleaseMode() => true;
  @override
  bool isProfileMode() => false;
}

class _MemoryTransport implements Transport {
  final events = <Map<String, dynamic>>[];
  final _first = Completer<void>();
  final _added = StreamController<void>.broadcast();

  Future<void> first() => _first.future;

  Future<void> count(int n) async {
    if (events.length >= n) return;
    await _added.stream.firstWhere((_) => events.length >= n);
  }

  @override
  Future<SentryId?> send(SentryEnvelope envelope) async {
    for (final item in envelope.items) {
      if (item.header.type != 'event') continue;
      final bytes = await item.dataFactory();
      events.add(jsonDecode(utf8.decode(bytes)) as Map<String, dynamic>);
      if (!_first.isCompleted) _first.complete();
      _added.add(null);
    }
    return envelope.header.eventId;
  }
}
