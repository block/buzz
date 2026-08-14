import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/channels/channel_sections/channel_sections_provider.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_storage.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

/// Cold-start regression for the one-time legacy migration.
///
/// `activeCommunityProvider` is a FutureProvider, so `.value` is null on the
/// first mount. The old `relayConfig.baseUrl` fallback consumed the migration
/// under a config-derived key; when the community later resolved to a
/// different origin the real key was empty and the legacy blob was gone.
void main() {
  test('does not consume the legacy migration until the community relay URL '
      'is known', () async {
    final keys = nostr.Keys.generate();
    const configUrl = 'https://config.example';
    const communityUrl = 'wss://community.example';
    final community = Community.create(
      name: 'Lit Box',
      relayUrl: communityUrl,
      nsec: keys.nsec,
    );

    SharedPreferences.setMockInitialValues({
      legacyChannelSectionsKey(keys.public): jsonEncode({
        'version': 1,
        'sections': [
          {'id': 's1', 'name': 'estimates', 'order': 0},
        ],
        'assignments': {'chan-1': 's1'},
      }),
    });
    final prefs = await SharedPreferences.getInstance();
    final communityReady = Completer<Community?>();

    final container = ProviderContainer(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        relayConfigProvider.overrideWith(
          () => _FakeRelayConfig(nsec: keys.nsec, baseUrl: configUrl),
        ),
        relaySessionProvider.overrideWith(_FakeRelaySession.new),
        activeCommunityProvider.overrideWith((ref) => communityReady.future),
      ],
    );
    addTearDown(container.dispose);

    final subscription = container.listen(channelSectionsProvider, (_, _) {});
    addTearDown(subscription.close);

    final cold = container.read(channelSectionsProvider);
    expect(cold.isReady, isFalse);
    expect(cold.store.sections, isEmpty);
    expect(prefs.getString(legacyChannelSectionsKey(keys.public)), isNotNull);
    expect(prefs.getString(channelSectionsKey(keys.public, configUrl)), isNull);
    expect(
      prefs.getString(channelSectionsKey(keys.public, communityUrl)),
      isNull,
    );

    communityReady.complete(community);
    await container.read(activeCommunityProvider.future);
    for (
      var i = 0;
      i < 20 && !container.read(channelSectionsProvider).isReady;
      i++
    ) {
      await Future<void>.delayed(Duration.zero);
    }

    final ready = container.read(channelSectionsProvider);
    expect(ready.isReady, isTrue);
    expect(ready.store.sections.single.name, 'estimates');
    expect(prefs.getString(legacyChannelSectionsKey(keys.public)), isNull);
    expect(
      prefs.getString(channelSectionsKey(keys.public, communityUrl)),
      isNotNull,
    );
    expect(prefs.getString(channelSectionsKey(keys.public, configUrl)), isNull);
  });
}

class _FakeRelayConfig extends RelayConfigNotifier {
  _FakeRelayConfig({required this.nsec, required this.baseUrl});

  final String nsec;
  final String baseUrl;

  @override
  RelayConfig build() => RelayConfig(baseUrl: baseUrl, nsec: nsec);
}

class _FakeRelaySession extends RelaySessionNotifier {
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [];

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async => () {};
}
