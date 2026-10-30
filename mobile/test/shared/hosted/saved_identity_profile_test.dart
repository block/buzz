import 'dart:convert';

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

  for (final invalid in [false, true]) {
    test(
      'loads only the saved key profile; invalid signature: $invalid',
      () async {
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
        FlutterSecureStorage.setMockInitialValues({
          'buzz_communities': records,
        });
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
                return http.Response(jsonEncode([event]), 200);
              }),
            ),
          ],
        );
        addTearDown(container.dispose);
        final profile = await container.read(
          savedIdentityProfileProvider(keys.public).future,
        );
        expect(requests, ['offline.example', 'home.example']);
        if (invalid) {
          expect(profile, isNull);
        } else {
          expect(profile?.displayName, 'Real name');
          expect(profile?.avatarUrl, 'https://home.example/avatar.png');
        }
        expect(
          await const FlutterSecureStorage().read(key: 'buzz_communities'),
          records,
        );
      },
    );
  }
}
