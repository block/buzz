import 'dart:async';
import 'dart:convert';

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
import 'package:sentry_flutter/sentry_flutter.dart';
// The SDK's own processors and integrations, so tests see what it really
// sends rather than hand-built events.
// ignore: implementation_imports
import 'package:sentry_flutter/src/breadcrumbs/debug_print_integration.dart';
// ignore: implementation_imports
import 'package:sentry_flutter/src/exception/flutter_error_integration.dart';
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
        'a tag not on the list is dropped',
        () async => (await event(
          SentryEvent(tags: {'operation': 'relayFrame', 'channel': 'x'}),
        )).tags,
        {'operation': 'relayFrame'},
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
      options
        ..transport = transport
        ..sendClientReports = false
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

  Future<void> first() => _first.future;

  @override
  Future<SentryId?> send(SentryEnvelope envelope) async {
    for (final item in envelope.items) {
      if (item.header.type != 'event') continue;
      final bytes = await item.dataFactory();
      events.add(jsonDecode(utf8.decode(bytes)) as Map<String, dynamic>);
      if (!_first.isCompleted) _first.complete();
    }
    return envelope.header.eventId;
  }
}
