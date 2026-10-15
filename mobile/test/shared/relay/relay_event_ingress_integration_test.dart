import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:buzz/shared/relay/relay_session.dart';

void main() {
  test('real HTTP and WebSocket ingress admits only verified events', () async {
    final queryEvent = _signedEvent('query');
    final historyEvent = _signedEvent('history');
    final liveEvent = _signedEvent('live');
    final relay = await _AdversarialRelay.start(
      queryEvents: [_forgedEvent('forged query'), queryEvent.toJson()],
      historyEvents: [_forgedEvent('forged history'), historyEvent.toJson()],
    );
    addTearDown(relay.close);

    final keys = nostr.Keys.generate();
    final session = RelaySessionNotifier();
    final container = ProviderContainer(
      overrides: [
        authProvider.overrideWith(() => _AuthenticatedAuthNotifier()),
        relayConfigProvider.overrideWith(
          () => _TestRelayConfigNotifier(relay.origin, keys.nsec),
        ),
        relaySessionProvider.overrideWith(() => session),
      ],
    );
    addTearDown(container.dispose);
    await container.read(authProvider.future);
    final listener = container.listen(relaySessionProvider, (_, _) {});
    addTearDown(listener.close);
    await _waitUntil(
      () =>
          container.read(relaySessionProvider).status ==
          SessionStatus.connected,
    );

    expect(await session.queryRelay(const []), [queryEvent]);
    expect(await session.fetchHistory(_filter), [historyEvent]);

    final delivered = <NostrEvent>[];
    final unsubscribe = await session.subscribe(_filter, delivered.add);
    addTearDown(unsubscribe);
    await relay.sendLive([_forgedEvent('forged live'), liveEvent.toJson()]);
    await _waitUntil(() => delivered.isNotEmpty);

    expect(delivered, [liveEvent]);
  });
}

class _AdversarialRelay {
  _AdversarialRelay._(this._server, this.queryEvents, this.historyEvents);

  final HttpServer _server;
  final List<Object?> queryEvents;
  final List<Object?> historyEvents;
  final List<WebSocket> _sockets = [];
  final Completer<String> _liveSubscription = Completer();

  String get origin => 'http://${_server.address.address}:${_server.port}';

  static Future<_AdversarialRelay> start({
    required List<Object?> queryEvents,
    required List<Object?> historyEvents,
  }) async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final relay = _AdversarialRelay._(server, queryEvents, historyEvents);
    server.listen(relay._handleRequest);
    return relay;
  }

  Future<void> _handleRequest(HttpRequest request) async {
    if (WebSocketTransformer.isUpgradeRequest(request)) {
      final socket = await WebSocketTransformer.upgrade(request);
      _sockets.add(socket);
      socket.add(jsonEncode(['AUTH', 'local-test-challenge']));
      socket.listen((raw) => _handleSocketMessage(socket, raw));
      return;
    }
    if (request.method == 'POST' && request.uri.path == '/query') {
      await request.drain<void>();
      request.response
        ..statusCode = HttpStatus.ok
        ..headers.contentType = ContentType.json
        ..write(jsonEncode(queryEvents));
      await request.response.close();
      return;
    }
    request.response.statusCode = HttpStatus.notFound;
    await request.response.close();
  }

  void _handleSocketMessage(WebSocket socket, Object? raw) {
    if (raw is! String) return;
    final decoded = jsonDecode(raw);
    if (decoded is! List || decoded.isEmpty) return;
    switch (decoded.first) {
      case 'AUTH':
        final event = decoded[1] as Map<String, dynamic>;
        socket.add(jsonEncode(['OK', event['id'], true, 'authenticated']));
      case 'REQ':
        final subId = decoded[1] as String;
        if (subId.startsWith('h-')) {
          for (final event in historyEvents) {
            socket.add(jsonEncode(['EVENT', subId, event]));
          }
          socket.add(jsonEncode(['EOSE', subId]));
        } else {
          if (!_liveSubscription.isCompleted) {
            _liveSubscription.complete(subId);
          }
          socket.add(jsonEncode(['EOSE', subId]));
        }
    }
  }

  Future<void> sendLive(List<Object?> events) async {
    final subId = await _liveSubscription.future;
    final socket = _sockets.single;
    for (final event in events) {
      socket.add(jsonEncode(['EVENT', subId, event]));
    }
  }

  Future<void> close() async {
    for (final socket in _sockets) {
      await socket.close();
    }
    await _server.close(force: true);
  }
}

class _AuthenticatedAuthNotifier extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.authenticated);
}

class _TestRelayConfigNotifier extends RelayConfigNotifier {
  _TestRelayConfigNotifier(this._origin, this._nsec);

  final String _origin;
  final String _nsec;

  @override
  RelayConfig build() => RelayConfig(baseUrl: _origin, nsec: _nsec);
}

const _channelId = '11111111-1111-4111-8111-111111111111';
const _filter = NostrFilter(
  kinds: EventKind.channelEventKinds,
  tags: {
    '#h': [_channelId],
  },
  limit: 50,
);

NostrEvent _signedEvent(String content) {
  final event = nostr.Event.from(
    kind: EventKind.streamMessageV2,
    content: content,
    tags: const [
      ['h', _channelId],
    ],
    secretKey: '1'.padLeft(64, '0'),
  );
  return NostrEvent.fromJson(event.toMap());
}

Map<String, dynamic> _forgedEvent(String content) => {
  ..._signedEvent(content).toJson(),
  'sig': List.filled(64, '00').join(),
};

Future<void> _waitUntil(bool Function() predicate) async {
  final deadline = DateTime.now().add(const Duration(seconds: 3));
  while (!predicate()) {
    if (DateTime.now().isAfter(deadline)) {
      fail('Timed out waiting for local relay workflow');
    }
    await Future<void>.delayed(const Duration(milliseconds: 5));
  }
}
