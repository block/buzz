import 'dart:convert';

import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  test(
    'publishes a signed kind:0 profile and preserves existing fields',
    () async {
      final keys = nostr.Keys.generate();
      http.Request? captured;
      final client = http_testing.MockClient((request) async {
        captured = request;
        return http.Response(jsonEncode({'accepted': true}), 200);
      });
      addTearDown(client.close);

      await publishProfileOverHttp(
        client: client,
        relayUrl: 'wss://relay.example.com/tenant-path?ignored=true',
        nsec: keys.nsec,
        displayName: '  Matt Example  ',
        existing: const UserProfile(
          pubkey: 'old',
          avatarUrl: 'https://cdn.example/avatar.png',
          about: 'Builder',
          nip05Handle: 'matt@example.com',
        ),
      );

      expect(captured?.url.toString(), 'https://relay.example.com/events');
      expect(captured?.headers['Authorization'], startsWith('Nostr '));
      expect(captured?.followRedirects, isFalse);
      final event = jsonDecode(captured!.body) as Map<String, dynamic>;
      expect(event['kind'], 0);
      expect(event['pubkey'], keys.public);
      final content =
          jsonDecode(event['content'] as String) as Map<String, dynamic>;
      expect(content, {
        'name': 'Matt Example',
        'display_name': 'Matt Example',
        'picture': 'https://cdn.example/avatar.png',
        'about': 'Builder',
        'nip05': 'matt@example.com',
      });
    },
  );

  test(
    'manual presence persists until Online restores automatic mode',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      var container = _buildContainer(prefs);

      expect(
        await container
            .read(presenceProvider.future)
            .timeout(
              const Duration(seconds: 2),
              onTimeout: () =>
                  throw StateError('initial presence did not resolve'),
            ),
        'online',
      );
      await container
          .read(presenceProvider.notifier)
          .setPresence('away')
          .timeout(
            const Duration(seconds: 2),
            onTimeout: () => throw StateError('setting Away did not resolve'),
          );
      expect(container.read(presenceProvider).value, 'away');
      expect(prefs.getString('buzz_presence_preference_aabb'), 'away');

      container.dispose();
      container = _buildContainer(prefs);
      addTearDown(container.dispose);
      expect(
        await container
            .read(presenceProvider.future)
            .timeout(
              const Duration(seconds: 2),
              onTimeout: () =>
                  throw StateError('stored presence did not resolve'),
            ),
        'away',
      );

      await container
          .read(presenceProvider.notifier)
          .setPresence('online')
          .timeout(
            const Duration(seconds: 2),
            onTimeout: () => throw StateError('setting Online did not resolve'),
          );
      expect(container.read(presenceProvider).value, 'online');
      expect(prefs.getString('buzz_presence_preference_aabb'), 'auto');
    },
  );
}

ProviderContainer _buildContainer(SharedPreferences prefs) => ProviderContainer(
  overrides: [
    savedPrefsProvider.overrideWithValue(prefs),
    myPubkeyProvider.overrideWithValue('aabb'),
    profileProvider.overrideWith(_FakeProfileNotifier.new),
    relaySessionProvider.overrideWith(_DisconnectedRelaySession.new),
    appLifecycleProvider.overrideWith(_ResumedLifecycle.new),
  ],
);

class _FakeProfileNotifier extends ProfileNotifier {
  @override
  Future<UserProfile?> build() async =>
      const UserProfile(pubkey: 'aabb', displayName: 'Test');
}

class _DisconnectedRelaySession extends RelaySessionNotifier {
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);
}

class _ResumedLifecycle extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}
