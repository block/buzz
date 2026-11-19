import 'package:buzz/features/channels/thread_follows/thread_follows_provider.dart';
import 'package:buzz/features/channels/thread_follows/thread_follows_storage.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

class _SwitchableRelayConfig extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(baseUrl: 'https://relay-a.example');

  void switchTo(String baseUrl) => state = RelayConfig(baseUrl: baseUrl);
}

void main() {
  test(
    'following threads in one community keeps the other\'s follows',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final config = _SwitchableRelayConfig();
      final container = ProviderContainer(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          relayConfigProvider.overrideWith(() => config),
          myPubkeyProvider.overrideWithValue('pk_a'),
        ],
      );
      addTearDown(container.dispose);
      ThreadFollowsNotifier notifier() =>
          container.read(threadFollowsProvider.notifier);
      bool following(String rootId) =>
          container.read(threadFollowsProvider).isFollowing(rootId);

      notifier().followThread('root-a');

      // A full cap in the second community must not evict the first's follows.
      config.switchTo('https://relay-b.example');
      expect(following('root-a'), isFalse);
      for (var i = 0; i < threadFollowsMaxEntries + 1; i++) {
        notifier().followThread('root-b-$i');
      }

      config.switchTo('https://relay-a.example');
      expect(following('root-a'), isTrue);
    },
  );
}
