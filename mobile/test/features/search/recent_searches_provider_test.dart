import 'package:buzz/features/search/recent_searches_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

class _FixedRelayConfigNotifier extends RelayConfigNotifier {
  final RelayConfig _config;

  _FixedRelayConfigNotifier(this._config);

  @override
  RelayConfig build() => _config;
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  Future<ProviderContainer> containerWithPrefs({
    required String relayUrl,
    required String? pubkey,
  }) async {
    final prefs = await SharedPreferences.getInstance();
    final container = ProviderContainer(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        relayConfigProvider.overrideWith(
          () => _FixedRelayConfigNotifier(RelayConfig(baseUrl: relayUrl)),
        ),
        myPubkeyProvider.overrideWithValue(pubkey),
      ],
    );
    addTearDown(container.dispose);
    return container;
  }

  test(
    'normalizes, deduplicates, caps, and persists submitted queries',
    () async {
      SharedPreferences.setMockInitialValues({});
      final first = await containerWithPrefs(
        relayUrl: 'https://relay-a.example',
        pubkey: 'pk-a',
      );
      final notifier = first.read(recentSearchesProvider.notifier);

      notifier.record('  Design  ');
      notifier.record('design');
      notifier.record('');
      for (var index = 0; index < 6; index++) {
        notifier.record('query-$index');
      }

      expect(first.read(recentSearchesProvider), [
        'query-5',
        'query-4',
        'query-3',
        'query-2',
        'query-1',
        'query-0',
      ]);

      final restarted = await containerWithPrefs(
        relayUrl: 'https://relay-a.example',
        pubkey: 'pk-a',
      );
      expect(
        restarted.read(recentSearchesProvider),
        first.read(recentSearchesProvider),
      );
    },
  );

  test('isolates persisted history by community and account', () async {
    SharedPreferences.setMockInitialValues({});
    final accountA = await containerWithPrefs(
      relayUrl: 'https://relay-a.example',
      pubkey: 'pk-a',
    );
    accountA.read(recentSearchesProvider.notifier).record('private query');

    final accountB = await containerWithPrefs(
      relayUrl: 'https://relay-a.example',
      pubkey: 'pk-b',
    );
    expect(accountB.read(recentSearchesProvider), isEmpty);
    accountB.read(recentSearchesProvider.notifier).record('account b query');

    final communityB = await containerWithPrefs(
      relayUrl: 'https://relay-b.example',
      pubkey: 'pk-a',
    );
    expect(communityB.read(recentSearchesProvider), isEmpty);
    communityB
        .read(recentSearchesProvider.notifier)
        .record('community b query');

    final accountAAgain = await containerWithPrefs(
      relayUrl: 'https://relay-a.example',
      pubkey: 'pk-a',
    );
    expect(accountAAgain.read(recentSearchesProvider), ['private query']);
  });
}
