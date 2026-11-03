import 'package:buzz/features/channels/bestie_welcome_storage.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../../shared/me/me_sync_test.dart' show FakeRelay;

void main() {
  test(
    'welcome survives a fresh provider and stays account/community/chat local',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final relay = FakeRelay();
      MeSync scope(String host, int identity) => MeSync(
        relay,
        RelayConfig(
          baseUrl: 'https://$host.test',
          nsec: nostr.Nip19.encode(
            prefix: nostr.Nip19Prefix.nsec,
            data: identity.toRadixString(16).padLeft(64, '0'),
          ),
        ),
        connected: false,
      );
      String? read(MeSync scope, String channel) {
        final container = ProviderContainer(
          overrides: [
            meSyncProvider.overrideWithValue(scope),
            savedPrefsProvider.overrideWithValue(prefs),
          ],
        );
        try {
          return container.read(bestieWelcomeProvider(channel));
        } finally {
          container.dispose();
        }
      }

      final original = scope('first', 1);
      await saveBestieWelcome(prefs, original, 'chat', 'agent');
      await prefs.reload();
      expect(read(scope('first', 1), 'chat'), 'agent');
      expect(read(scope('second', 1), 'chat'), isNull);
      expect(read(scope('first', 2), 'chat'), isNull);
      expect(read(scope('first', 1), 'other-chat'), isNull);
      expect(relay.writes, 0);
      expect(relay.directoryQueries, 0);
    },
  );
}
