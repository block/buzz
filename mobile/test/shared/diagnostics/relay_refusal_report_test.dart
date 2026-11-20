import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/diagnostics/diagnostic_startup.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:sentry_flutter/sentry_flutter.dart';

/// Refusals produced by the relay code itself, reported the way an uncaught
/// error reaches the SDK, and read back as the serialized event.
void main() {
  late _MemoryTransport transport;
  late Hub hub;

  setUp(() {
    transport = _MemoryTransport();
    final options = SentryFlutterOptions();
    configureDiagnostics(options, 'https://key@example.org/1');
    options
      ..transport = transport
      ..sendClientReports = false;
    hub = Hub(options);
  });
  tearDown(() => hub.close());

  Future<Map<String, dynamic>> escaped(Object error, StackTrace stack) async {
    await hub.captureException(error, stackTrace: stack);
    return transport.events.single;
  }

  Future<(Object, StackTrace)> failure(Future<Object?> call) async {
    try {
      await call;
    } catch (error, stack) {
      return (error, stack);
    }
    fail('the call succeeded');
  }

  group('a refused /query', () {
    final keys = nostr.Keys.generate();
    const marker = 'private-marker';

    RelaySessionNotifier session(http.Client client) {
      final session = RelaySessionNotifier(httpClient: client);
      final container = ProviderContainer(
        overrides: [
          authProvider.overrideWith(_PendingAuth.new),
          relayConfigProvider.overrideWith(() => _Config(keys.nsec)),
          relaySessionProvider.overrideWith(() => session),
        ],
      );
      addTearDown(container.dispose);
      container.read(relaySessionProvider);
      return session;
    }

    for (final (status, reason) in [
      (429, 'rate-limited'),
      (401, 'auth-required'),
      (403, 'restricted'),
      (503, 'error'),
      (400, 'other'),
    ]) {
      test('$status is tagged query and $reason, without its body', () async {
        final relay = session(
          MockClient((_) async => http.Response('{"error":"$marker"}', status)),
        );

        final (error, stack) = await failure(relay.queryRelay(const []));
        final sent = await escaped(error, stack);

        expect(error, isA<RelayException>());
        expect((error as RelayException).statusCode, status);
        expect(error.body, contains(marker));
        expect(sent['tags']['relay.call'], 'query');
        expect(sent['tags']['relay.reason'], reason);
        expect(sent['fingerprint'], [
          '{{ default }}',
          'relay.call:query',
          'relay.reason:$reason',
        ]);
        expect(jsonEncode(sent), isNot(contains(marker)));
      });
    }

    group('of the read state snapshot', () {
      final discovery = jsonEncode({
        'read_state_snapshot': {
          'version': 1,
          'community_id': 'abcdabcd-eeee-abcd-abcd-abcdefabcdef',
          'max_events': 4096,
          'max_event_array_bytes': 8 * 1024 * 1024,
        },
      });

      test('is tagged query and its reason, without its body', () async {
        final relay = session(
          MockClient(
            (request) async => request.method == 'GET'
                ? http.Response(discovery, 200)
                : http.Response('{"error":"$marker"}', 429),
          ),
        );

        final (error, stack) = await failure(relay.fetchReadStateSnapshot());
        final sent = await escaped(error, stack);

        expect((error as RelayException).statusCode, 429);
        expect(sent['tags']['relay.call'], 'query');
        expect(sent['tags']['relay.reason'], 'rate-limited');
        expect(jsonEncode(sent), isNot(contains(marker)));
      });

      test('discovery is not a query and is not tagged', () async {
        final relay = session(
          MockClient((_) async => http.Response('{"error":"$marker"}', 429)),
        );

        final (error, stack) = await failure(relay.fetchReadStateSnapshot());
        final sent = await escaped(error, stack);

        expect(error, isA<RelayException>());
        expect(error, isNot(isA<RelayCallError>()));
        expect(sent['tags'], isNot(contains('relay.call')));
        expect(jsonEncode(sent), isNot(contains(marker)));
      });
    });
  });

  group('a one-shot signed event', () {
    const marker = 'private-marker';

    var eventsReceived = 0;

    Future<HttpServer> relayAnswering(
      String? reason, {
      bool acknowledgeAuth = true,
    }) async {
      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      addTearDown(() => server.close(force: true));
      server.listen((request) async {
        final ws = await WebSocketTransformer.upgrade(request);
        ws.add(jsonEncode(['AUTH', 'challenge']));
        ws.listen((raw) {
          final frame = jsonDecode(raw as String) as List<dynamic>;
          final id = (frame[1] as Map<String, dynamic>)['id'];
          if (frame[0] == 'AUTH' && acknowledgeAuth) {
            ws.add(jsonEncode(['OK', id, true, '']));
          }
          if (frame[0] == 'EVENT') eventsReceived++;
          if (frame[0] == 'EVENT' && reason != null) {
            ws.add(jsonEncode(['OK', id, false, reason]));
          }
        });
      });
      return server;
    }

    Future<void> submit(HttpServer server, {Duration? timeout}) =>
        _submitFromTheCaller(server, timeout: timeout);

    test('refused is tagged publish and the relay prefix, '
        'with the caller\'s stack and without the relay text', () async {
      final server = await relayAnswering('rate-limited: $marker');

      final (error, stack) = await failure(submit(server));
      final sent = await escaped(error, stack);

      expect(error, isA<RelayEventRejectedException>());
      expect(
        error.toString(),
        'Exception: Relay rejected event: rate-limited: $marker',
      );
      expect((error as RelayCallError).call, RelayCall.publish);
      expect(error.reason, RelayReason.rateLimited);
      expect(stack.toString(), contains('_submitFromTheCaller'));
      expect(sent['tags']['relay.call'], 'publish');
      expect(sent['tags']['relay.reason'], 'rate-limited');
      expect(jsonEncode(sent), isNot(contains(marker)));
      expect(jsonEncode(sent), contains('_submitFromTheCaller'));
    });

    test('refused with an unknown prefix has the reason other', () async {
      final server = await relayAnswering('teapot: $marker');

      final (error, stack) = await failure(submit(server));
      final sent = await escaped(error, stack);

      expect(sent['tags']['relay.reason'], 'other');
      expect(jsonEncode(sent), isNot(contains(marker)));
    });

    test('unanswered is a publish timeout from the caller', () async {
      final server = await relayAnswering(null);

      final (error, stack) = await failure(
        submit(server, timeout: const Duration(milliseconds: 200)),
      );
      final sent = await escaped(error, stack);

      expect(error, isA<RelayTimeoutException>());
      expect(stack.toString(), contains('_submitFromTheCaller'));
      expect(sent['tags']['relay.call'], 'publish');
      expect(sent['tags']['relay.reason'], 'timeout');
    });

    test('an authentication the relay never acknowledges is a publish '
        'timeout from the caller, and the event is never sent', () async {
      eventsReceived = 0;
      final server = await relayAnswering(
        'rate-limited: $marker',
        acknowledgeAuth: false,
      );

      // The socket's 8 s authentication deadline passes first.
      final (error, stack) = await failure(submit(server));
      final sent = await escaped(error, stack);

      expect(error, isA<TimeoutException>());
      expect(error, isA<RelayTimeoutException>());
      expect(stack.toString(), contains('_submitFromTheCaller'));
      expect(sent['tags']['relay.call'], 'publish');
      expect(sent['tags']['relay.reason'], 'timeout');
      expect(jsonEncode(sent), isNot(contains(marker)));
      expect(eventsReceived, 0);
    }, timeout: const Timeout(Duration(seconds: 30)));
  });
}

Future<NostrEvent> _submitFromTheCaller(
  HttpServer server, {
  Duration? timeout,
}) => submitSignedEventOnce(
  wsUrl: 'ws://127.0.0.1:${server.port}',
  nsec: nostr.Keys.generate().nsec,
  kind: 1,
  content: '',
  tags: const [],
  timeout: timeout ?? const Duration(seconds: 12),
);

class _PendingAuth extends AuthNotifier {
  @override
  Future<AuthState> build() => Completer<AuthState>().future;
}

class _Config extends RelayConfigNotifier {
  _Config(this.nsec);

  final String nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: nsec);
}

class _MemoryTransport implements Transport {
  final events = <Map<String, dynamic>>[];

  @override
  Future<SentryId?> send(SentryEnvelope envelope) async {
    for (final item in envelope.items) {
      if (item.header.type != 'event') continue;
      final bytes = await item.dataFactory();
      events.add(jsonDecode(utf8.decode(bytes)) as Map<String, dynamic>);
    }
    return envelope.header.eventId;
  }
}
