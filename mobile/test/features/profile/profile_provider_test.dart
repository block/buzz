import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
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
