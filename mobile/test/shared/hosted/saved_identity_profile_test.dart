import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:buzz/shared/hosted/saved_identity_profile.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final secret = '1'.padLeft(64, '0');
  final keys = nostr.Keys(secret);
  final other = nostr.Keys('2'.padLeft(64, '0'));

  for (final failure in ['oversized', 'stalled body', 'stalled headers']) {
    testWidgets('closes $failure before trying another saved relay', (
      tester,
    ) async {
      FlutterSecureStorage.setMockInitialValues({
        'buzz_communities': jsonEncode([
          for (final host in ['broken.example', 'next.example'])
            {
              'name': host,
              'relayUrl': 'wss://$host',
              'pubkey': keys.public,
              'nsec': keys.nsec,
            },
        ]),
      });
      var cancelled = false;
      bool? closedBeforeFallback;
      bool? cancelledBeforeFallback;
      final body = StreamController<List<int>>(
        onCancel: () {
          cancelled = true;
        },
      );
      final clients = <_ProfileClient>[];
      final container = ProviderContainer(
        overrides: [
          savedIdentityProfileClientProvider.overrideWithValue(() {
            final client = _ProfileClient((request) async {
              if (request.url.host == 'next.example') {
                closedBeforeFallback = clients.first.closed;
                cancelledBeforeFallback = cancelled;
                return http.StreamedResponse(
                  Stream.value(utf8.encode('[]')),
                  503,
                );
              }
              if (failure == 'stalled headers') {
                return Completer<http.StreamedResponse>().future;
              }
              if (failure == 'oversized') {
                body.add(List.filled(256 * 1024, 32));
                body.add([32]);
              }
              return http.StreamedResponse(body.stream, 200);
            });
            clients.add(client);
            return client;
          }),
        ],
      );
      addTearDown(container.dispose);
      final subscription = container.listen(
        savedIdentityProfileProvider(keys.public),
        (_, _) {},
      );
      addTearDown(subscription.close);
      var finished = false;
      container.read(savedIdentityProfileProvider(keys.public).future).then((
        value,
      ) {
        expectSync(value, isNull);
        finished = true;
      });
      await tester.pump();
      if (failure == 'oversized') expectSync(finished, isTrue);
      await tester.pump(const Duration(seconds: 6));
      expectSync(finished, isTrue);
      expectSync(clients, hasLength(2));
      expectSync(closedBeforeFallback, isTrue);
      if (failure != 'stalled headers') {
        expectSync(cancelledBeforeFallback, isTrue);
      }
      expectSync(clients.every((client) => client.closed), isTrue);
      unawaited(body.close());
    });
  }

  for (final variant in ['valid', 'invalid signature', 'raw gzip']) {
    final invalid = variant == 'invalid signature';
    test('loads only the saved key profile: $variant', () async {
      final records = jsonEncode([
        {
          'name': 'Other',
          'relayUrl': 'wss://other.example',
          'pubkey': other.public,
          'nsec': other.nsec,
        },
        {
          'name': 'Unavailable',
          'relayUrl': 'wss://offline.example',
          'pubkey': keys.public,
          'nsec': keys.nsec,
        },
        {
          'name': 'Home',
          'relayUrl': 'wss://home.example',
          'pubkey': keys.public,
          'nsec': keys.nsec,
        },
      ]);
      FlutterSecureStorage.setMockInitialValues({'buzz_communities': records});
      final event =
          jsonDecode(
                signNostrEvent(
                  kind: 0,
                  secretKey: secret,
                  content: jsonEncode({
                    'name': 'Real name',
                    'picture': 'https://home.example/avatar.png',
                  }),
                ).toJson(),
              )
              as Map<String, dynamic>;
      if (invalid) event['sig'] = '0' * 128;
      final requests = <String>[];
      final container = ProviderContainer(
        overrides: [
          savedIdentityProfileClientProvider.overrideWithValue(
            () => MockClient((request) async {
              requests.add(request.url.host);
              expect(request.followRedirects, isFalse);
              expect(request.headers['Authorization'], startsWith('Nostr '));
              expect(jsonDecode(request.body)[0]['authors'], [keys.public]);
              if (request.url.host == 'offline.example') {
                return http.Response('', 503);
              }
              final json = jsonEncode([event]);
              if (variant == 'raw gzip') {
                return http.Response.bytes(gzip.encode(utf8.encode(json)), 200);
              }
              return http.Response(json, 200);
            }),
          ),
        ],
      );
      addTearDown(container.dispose);
      final profile = await container.read(
        savedIdentityProfileProvider(keys.public).future,
      );
      expect(requests, ['offline.example', 'home.example']);
      if (invalid || variant == 'raw gzip') {
        expect(profile, isNull);
      } else {
        expect(profile?.displayName, 'Real name');
        expect(profile?.avatarUrl, 'https://home.example/avatar.png');
      }
      expect(
        await const FlutterSecureStorage().read(key: 'buzz_communities'),
        records,
      );
    });
  }
}

class _ProfileClient extends http.BaseClient {
  _ProfileClient(this.respond);
  final Future<http.StreamedResponse> Function(http.BaseRequest) respond;
  bool closed = false;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) =>
      respond(request);

  @override
  void close() {
    closed = true;
  }
}
