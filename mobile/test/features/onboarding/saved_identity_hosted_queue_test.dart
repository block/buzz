import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/onboarding/self_hosted_entry_sheet.dart';
import 'package:buzz/features/onboarding/saved_identity_card.dart';
import 'package:buzz/shared/hosted/saved_identity_profile.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  testWidgets(
    'hosted chooser never queues passive account work',
    (tester) async {
      final keys = [
        for (var i = 1; i <= 4; i++)
          nostr.Keys(i.toRadixString(16).padLeft(64, '0')),
      ];
      final relay = 'wss://saved.${HostedEnvironment.communitySuffix}';
      FlutterSecureStorage.setMockInitialValues({
        'buzz_communities': jsonEncode([
          for (final key in keys)
            {
              'name': 'Saved',
              'relayUrl': relay,
              'pubkey': key.public,
              'nsec': key.nsec,
            },
        ]),
      });
      final release = Completer<void>();
      final started = Completer<void>();
      final paths = <String>[];
      final expiry = DateTime.now()
          .add(const Duration(hours: 1))
          .toIso8601String();
      final account = BuilderlabAccountClient(
        httpClient: MockClient((request) async {
          final path = request.url.path;
          paths.add(path);
          if (path.endsWith('/known-communities/list')) {
            started.complete();
            await release.future;
          }
          final value = path.endsWith('/login/exchange')
              ? {'session_credential': 'test-session', 'expires_at': expiry}
              : path.endsWith('/auth/me')
              ? {'subject': 'test-subject', 'expires_at': expiry}
              : path.endsWith('/nostr-identities/current')
              ? {
                  'identity': {'pubkey_hex': keys.first.public},
                }
              : path.endsWith('/identity/assertions')
              ? {
                  'assertion': 'test-assertion',
                  'nostr_pubkey': keys.first.public,
                  'expires_at':
                      DateTime.now().millisecondsSinceEpoch ~/ 1000 + 120,
                }
              : <String, dynamic>{};
          return http.Response(jsonEncode(value), 200);
        }),
      );
      addTearDown(account.dispose);
      await account.exchange(code: 'code', verifier: 'v' * 43);
      paths.clear();
      final busy = account.post('/v1/buzz/known-communities/list', {});
      await started.future;
      final container = ProviderContainer(
        overrides: [
          builderlabAccountProvider.overrideWithValue(account),
          savedIdentityProfileClientProvider.overrideWithValue(
            () => MockClient((_) async {
              fail('Uncached hosted cards must not send relay requests');
            }),
          ),
        ],
      );
      addTearDown(container.dispose);
      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: container,
          child: const MaterialApp(
            home: SelfHostedEntrySheet(
              relay: 'wss://join.example',
              name: 'Join',
              fullScreen: true,
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byType(SavedIdentityCard), findsNWidgets(4));
      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: container,
          child: const SizedBox(),
        ),
      );
      await tester.pumpAndSettle();
      final active = account.post('/v1/buzz/known-communities/add', {});
      release.complete();
      await busy;
      await active;
      await tester.pumpAndSettle();
      expect(paths.map((path) => path.split('/').last), ['list', 'add']);

      // Active authentication may populate the cache; passive reads use it only
      // for the same relay, key, account session, and unexpired lifetime.
      var now = DateTime.now();
      final auth = HostedRelayAuth(account, now: () => now);
      addTearDown(auth.dispose);
      final config = RelayConfig(baseUrl: relay, nsec: keys.first.nsec);
      expect(auth.cachedHeaders(config), isNull);
      final headers = await auth.headers(config);
      final count = paths.length;
      expect(auth.cachedHeaders(config), headers);
      expect(
        auth.cachedHeaders(RelayConfig(baseUrl: relay, nsec: keys.last.nsec)),
        isNull,
      );
      expect(
        auth.cachedHeaders(
          RelayConfig(
            baseUrl: 'wss://other.${HostedEnvironment.communitySuffix}',
            nsec: keys.first.nsec,
          ),
        ),
        isNull,
      );
      now = now.add(const Duration(minutes: 5));
      expect(auth.cachedHeaders(config), isNull);
      expect(paths.length, count);
    },
    skip:
        !const bool.fromEnvironment('BUZZ_HOSTED_RELAY_AUTH') ||
        !HostedEnvironment.enabled,
  );
}
