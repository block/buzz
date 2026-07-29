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

  test('an invite-joined community keeps its recent searches after '
      'origin canonicalization', () async {
    const legacyKey = 'recent_searches_v1:wss://relay-a.example:pk-a';
    const canonicalKey = 'recent_searches_v1:https://relay-a.example:pk-a';
    SharedPreferences.setMockInitialValues({
      legacyKey: <String>['nostr', 'relays'],
    });

    final container = await containerWithPrefs(
      relayUrl: 'wss://relay-a.example',
      pubkey: 'pk-a',
    );

    expect(container.read(recentSearchesProvider), ['nostr', 'relays']);

    await Future<void>.delayed(Duration.zero);
    final prefs = await SharedPreferences.getInstance();
    expect(prefs.getStringList(canonicalKey), ['nostr', 'relays']);
    expect(prefs.getStringList(legacyKey), isNull);
  });

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
