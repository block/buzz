import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/diagnostics/diagnostic_filter.dart';
import 'package:buzz/shared/diagnostics/diagnostic_member_binding.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/diagnostics/diagnostic_startup.dart';
import 'package:buzz/shared/diagnostics/sentry_diagnostic_reporter.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
// ignore: depend_on_referenced_packages, implementation_imports
import 'package:sentry/src/deduplication/deduplication_event_processor.dart';
// ignore: depend_on_referenced_packages, implementation_imports
import 'package:sentry/src/exception/dart_exception_type_identifier.dart';
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
        'a navigation breadcrumb that names no app screen is dropped',
        () async => crumb(Breadcrumb(category: 'navigation', message: 'Chat')),
        null,
      ),
      (
        'a navigation breadcrumb with an unlisted screen name is dropped',
        () async => crumb(
          Breadcrumb(category: 'navigation', data: {'screen': 'secret-room'}),
        ),
        null,
      ),
      (
        'a navigation breadcrumb keeps only its screen name',
        () async => crumb(
          Breadcrumb(
            category: 'navigation',
            message: 'secret-room',
            data: {'screen': 'channel', 'route': '/channels/secret-id'},
          ),
        )?.data,
        {'screen': 'channel'},
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
      (
        'a count keeps the install ID',
        () async => (await metric({
          'backend': SentryAttribute.string('relay.example.org'),
          'install.id': SentryAttribute.string('i'),
        })).keys.toSet(),
        {'backend', 'install.id'},
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
      // The processors and type identifier Sentry.init and SentryFlutter.init
      // install.
      options
        ..transport = transport
        ..sendClientReports = false
        ..prependExceptionTypeIdentifier(DartExceptionTypeIdentifier())
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
      (
        'a Flutter framework error',
        FlutterError('private-marker'),
        'AssertionError',
      ),
      ('an ordinary exception', Exception('private-marker'), 'Exception'),
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

    final kept = <(String, Object, String, String)>[
      (
        'a bad state',
        StateError('ref used after dispose\nprivate-marker'),
        'StateError',
        'Bad state: ref used after dispose',
      ),
      (
        'a bad state split by a carriage return',
        StateError('ref used after dispose\rprivate-marker'),
        'StateError',
        'Bad state: ref used after dispose',
      ),
      (
        'a failed assertion',
        AssertionError('list is sorted'),
        'AssertionError',
        'list is sorted',
      ),
      (
        'an unsupported call',
        UnsupportedError('no resize'),
        'Unsupported',
        'no resize',
      ),
      (
        'an unimplemented call',
        UnimplementedError('no export'),
        'Unimpl',
        'no export',
      ),
      (
        'a bad state naming nullable types',
        StateError("wanted List<String?> or (int?, String?), got 'int?'"),
        'StateError',
        "wanted List<String?> or (int?, String?), got 'int?'",
      ),
      (
        'a bad state naming providers',
        StateError(
          'The provider `NotifierProvider<Foo, Bar>#1a2b3` depends on '
          '`Provider<Baz?>#c0ffe`, which may be scoped.',
        ),
        'StateError',
        'The provider `NotifierProvider<Foo, Bar>` depends on '
            '`Provider<Baz?>`, which may be scoped.',
      ),
      (
        'a render box that was not laid out',
        StateError('RenderBox was not laid out: RenderFlex#1a2b3'),
        'StateError',
        'RenderBox was not laid out: RenderFlex',
      ),
      (
        'a bad state quoting a null-aware condition',
        StateError("'(link ?? false) || linkUrl == null': is it null? no."),
        'StateError',
        "'(link ?? false) || linkUrl == null': is it null? no.",
      ),
      (
        'a bad state naming a long identifier',
        StateError('_RecordingPublishRelaySessionNotifier was disposed'),
        'StateError',
        '_RecordingPublishRelaySessionNotifier was disposed',
      ),
      ('a failed cast', _failedCast(), 'TypeError', 'is not a subtype of'),
      // The accepted risk (ADR 0007): a short value any code interpolates is
      // sent as written when no pattern recognizes it.
      (
        'a bad state holding a short value',
        StateError('no session for hunter2'),
        'StateError',
        'no session for hunter2',
      ),
    ];
    for (final (name, error, type, text) in kept) {
      test('keeps the first line of the text of $name', () async {
        await hub.captureException(error, stackTrace: StackTrace.current);
        final sent = transport.events.single;

        final exception = (sent['exception']['values'] as List).last as Map;
        expect(exception['type'], contains(type));
        expect(exception['value'], contains(text));
        expect(exception['value'], isNot(contains('\n')));
        expect(exception['value'], isNot(contains('\r')));
        expect(jsonEncode(sent), isNot(contains('private-marker')));
      });
    }

    final replaced = <(String, String)>[
      ('a URL', 'open https://e.org/f?token=private-marker'),
      ('a URL with a quote', "open https://e.org/O'Brien?private-marker#x"),
      ('a URL inside a word', 'open id_https://e.org/f?private-marker'),
      ('a mail link', 'open mailto:a@e.org?subject=private-marker'),
      ('a fragment', 'open page#private-marker'),
      (
        'a mail link whose query starts with a quote',
        "open mailto:a@e.org?'subject=private-marker",
      ),
      (
        'a mail link whose query starts with a comma',
        'open mailto:a@e.org?,tag=private-marker',
      ),
      (
        'a phone link whose query starts with a bracket',
        'open tel:+1555?>private-marker',
      ),
      (
        'a path whose fragment starts with a quote',
        "open /settings#'private-marker",
      ),
      ('a word whose query starts with a quote', "open page?'private-marker"),
      (
        'a fragment that starts like an identity hash',
        'open /settings#abcde-private-marker',
      ),
      (
        'a provider with an argument',
        'The provider FutureProvider<int>#1a2b3(private-marker) was disposed',
      ),
      (
        'a private key',
        'Unexpected sign-in failure: nsec1${'q' * 52}7x private-marker',
      ),
      ('a hex key', 'no member ${'ab12' * 16} private-marker'),
      (
        'a token',
        'bad token eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0 private-marker',
      ),
      ('a long line', 'state ${'x' * 200} private-marker'),
      ('a very long line', 'state ${'x' * 500} private-marker'),
    ];
    for (final (name, text) in replaced) {
      test('replaces kept text that has $name', () async {
        await hub.captureException(
          StateError(text),
          stackTrace: StackTrace.current,
        );
        final sent = transport.events.single;

        final exception = (sent['exception']['values'] as List).last as Map;
        expect(exception['value'], DiagnosticFilter.removedText);
        expect(jsonEncode(sent), isNot(contains('private-marker')));
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

    test('a member change drops the previous member\'s trail', () async {
      DiagnosticBreadcrumb trail(String host) => DiagnosticBreadcrumb(
        NetworkEvent(
          backend: host,
          endpoint: NetworkEndpoint.relayConnection,
          outcome: NetworkOutcome.failure,
          failure: NetworkFailure.serverError,
        ),
      );
      List<Object?> backends(Map<String, dynamic> event) => [
        for (final crumb in (event['breadcrumbs'] as List? ?? const []))
          (crumb as Map)['data']?['backend'],
      ];
      final reporter = SentryDiagnosticReporter(hub: hub)
        ..leaveBreadcrumb(trail('before.example.org'))
        ..setMember('a' * 64)
        ..leaveBreadcrumb(trail('a.example.org'))
        ..reportError(
          StateError('failed'),
          StackTrace.current,
          operation: DiagnosticOperation.relayFrame,
        );
      await transport.first();
      reporter
        ..setMember('b' * 64)
        ..leaveBreadcrumb(trail('b.example.org'))
        ..reportError(
          StateError('failed'),
          StackTrace.current,
          operation: DiagnosticOperation.relayFrame,
        );
      await transport.count(2);
      reporter
        ..setMember(null)
        ..reportError(
          StateError('failed'),
          StackTrace.current,
          operation: DiagnosticOperation.relayFrame,
        );
      await transport.count(3);

      // Signing in keeps what came before it.
      expect(backends(transport.events[0]), [
        'before.example.org',
        'a.example.org',
      ]);
      expect(backends(transport.events[1]), ['b.example.org']);
      expect(backends(transport.events[2]), isEmpty);
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

    group('an error that escapes from a relay call', () {
      Future<Map<String, dynamic>> escaped(Object error) async {
        // As an uncaught asynchronous error reaches the SDK: straight to the
        // hub, not through the app's reporter.
        await hub.captureException(error, stackTrace: StackTrace.current);
        return transport.events.single;
      }

      test(
        'is tagged with the call and the reason class, not the text',
        () async {
          final sent = await escaped(
            const RelayEventRejectedException('rate-limited: private-marker'),
          );

          expect(sent['tags']['relay.call'], 'publish');
          expect(sent['tags']['relay.reason'], 'rate-limited');
          expect(jsonEncode(sent), isNot(contains('private-marker')));
          final exception = (sent['exception']['values'] as List).last as Map;
          expect(exception['type'], 'RelayEventRejectedException');
          expect(exception['value'], DiagnosticFilter.removedText);
        },
      );

      test('is grouped by call and reason on top of its stack', () async {
        final sent = await escaped(
          RelayClosedException(RelayCall.history, 'error: private-marker'),
        );

        expect(sent['tags']['relay.call'], 'history');
        expect(sent['tags']['relay.reason'], 'error');
        expect(sent['fingerprint'], [
          '{{ default }}',
          'relay.call:history',
          'relay.reason:error',
        ]);
      });

      test('names a timeout as one', () async {
        final sent = await escaped(
          RelayTimeoutException(RelayCall.query, 'private-marker'),
        );

        expect(sent['tags']['relay.call'], 'query');
        expect(sent['tags']['relay.reason'], 'timeout');
        expect(jsonEncode(sent), isNot(contains('private-marker')));
      });

      test('with an unknown prefix has the reason other', () async {
        final sent = await escaped(
          const RelayEventRejectedException('teapot: private-marker'),
        );

        expect(sent['tags']['relay.reason'], 'other');
        expect(jsonEncode(sent), isNot(contains('private-marker')));
      });

      test('is tagged when reported through the reporter too', () async {
        SentryDiagnosticReporter(hub: hub).reportError(
          const RelayEventRejectedException('invalid: private-marker'),
          StackTrace.current,
          operation: DiagnosticOperation.relayRejection,
        );
        await transport.first();

        final sent = transport.events.single;
        expect(sent['tags']['operation'], 'relayRejection');
        expect(sent['tags']['relay.call'], 'publish');
        expect(sent['tags']['relay.reason'], 'invalid');
      });

      test(
        'is the only kind of error that gets the tags or the grouping',
        () async {
          final sent = await escaped(StateError('private-marker'));

          expect(sent['tags'], isNot(contains('relay.call')));
          expect(sent['tags'], isNot(contains('relay.reason')));
          expect(sent, isNot(contains('fingerprint')));
        },
      );
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

Object _failedCast() {
  try {
    // ignore: unnecessary_cast
    return <Object>[1] as List<String>;
  } on TypeError catch (error) {
    return error;
  }
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
