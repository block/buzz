import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/diagnostics/diagnostic_member_binding.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/diagnostics/diagnostic_startup.dart';
import 'package:buzz/shared/diagnostics/sentry_diagnostic_reporter.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:sentry_flutter/sentry_flutter.dart';

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
        'a URL loses its query string and fragment',
        () async => (await event(
          SentryEvent(
            exceptions: [
              SentryException(
                type: 'StateError',
                value:
                    'failed https://e.org/p?token=abc#frag and wss://r.org/x?y=1',
              ),
            ],
          ),
        )).exceptions!.single.value,
        'failed https://e.org/p and wss://r.org/x',
      ),
      (
        'a parse failure loses the input it quotes',
        () async => (await event(
          SentryEvent(
            exceptions: [
              SentryException(
                type: 'FormatException',
                value: 'Unexpected character: {"content":"hello"}',
              ),
            ],
          ),
        )).exceptions!.single.value,
        'The input could not be parsed.',
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
      expect(options.replay.sessionSampleRate, isNull);
      expect(options.tracesSampleRate, isNull);
    });
  });

  test('a build with no reporting destination never starts the SDK', () async {
    DiagnosticReporter? started;
    await runWithDiagnostics(destination: '', app: (r) => started = r);

    expect(started, isA<NoopDiagnosticReporter>());
    expect(Sentry.isEnabled, isFalse);
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
