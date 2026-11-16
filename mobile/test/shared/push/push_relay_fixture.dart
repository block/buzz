import 'dart:convert';
import 'dart:io';
import 'package:buzz/shared/crypto/nip44.dart';
import 'package:nostr/nostr.dart' as nostr;

final relayKeys = nostr.Keys('2' * 64);

class DirectPushHttp extends HttpOverrides {}

class PushRelayFixture {
  PushRelayFixture(this.server, this.failMetadata);
  final HttpServer server;
  bool failMetadata;
  final sockets = <WebSocket>[];
  final leases = <Map<String, dynamic>>[];
  int nip11 = 0;
  String get httpUrl => 'http://127.0.0.1:${server.port}';
  String get wsUrl => 'ws://127.0.0.1:${server.port}';
  static Future<PushRelayFixture> start({bool failMetadata = false}) async {
    final r = PushRelayFixture(
      await HttpServer.bind(InternetAddress.loopbackIPv4, 0),
      failMetadata,
    );
    r.server.listen(r.handle);
    return r;
  }

  Future<void> handle(HttpRequest request) async {
    if (!WebSocketTransformer.isUpgradeRequest(request)) {
      nip11++;
      request.response.statusCode = failMetadata ? 503 : 200;
      request.response.headers.contentType = ContentType.json;
      request.response.write(
        jsonEncode(
          failMetadata
              ? {'error': 'fixture-unavailable'}
              : {
                  'supported_extensions': ['buzz-push-v1'],
                  'push': {
                    'origin': wsUrl,
                    'keys': [
                      {
                        'id': 'fixture',
                        'pubkey': relayKeys.public,
                        'current': true,
                      },
                    ],
                    'push_kinds': [9, 40002, 45001, 45003],
                    'h_grammar': 'uuid-lowercase',
                    'class_support': {
                      'apns': ['default'],
                    },
                    'limitation': {
                      'max_lease_ttl': 2592000,
                      'max_leases_per_pubkey': 16,
                      'max_subscriptions_per_lease': 16,
                      'max_kinds': 16,
                      'max_authors': 20,
                      'max_h': 50,
                      'max_tag_values': 20,
                      'max_ignore': 8,
                      'max_content_len': 65536,
                      'max_plaintext_len': 32768,
                      'max_endpoint_len': 4096,
                      'max_string_len': 512,
                    },
                  },
                },
        ),
      );
      await request.response.close();
      return;
    }
    final socket = await WebSocketTransformer.upgrade(request);
    sockets.add(socket);
    socket.listen((raw) {
      final frame = jsonDecode(raw as String) as List;
      final event = frame[1] as Map<String, dynamic>;
      if (frame[0] == 'EVENT') {
        final payload =
            jsonDecode(
                  nip44Decrypt(
                    getConversationKey(
                      relayKeys.secret,
                      event['pubkey'] as String,
                    ),
                    event['content'] as String,
                  ),
                )
                as Map<String, dynamic>;
        leases.add(payload);
      }
      socket.add(jsonEncode(['OK', event['id'], true, 'accepted-fixture']));
    });
    socket.add(jsonEncode(['AUTH', 'local-fixture-challenge']));
  }

  Future<void> close() async {
    for (final s in sockets) {
      await s.close();
    }
    await server.close(force: true);
  }
}
