import 'dart:async';

import 'package:buzz/features/channels/channel_mutes/channel_mutes_provider.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_provider.dart';
import 'package:buzz/features/channels/channel_sort/channel_sort_provider.dart';
import 'package:buzz/features/channels/channel_stars/channel_stars_provider.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/community_theme_provider.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  test('theme crypto and publishing use the normalized signing key', () async {
    SharedPreferences.setMockInitialValues({});
    final keys = nostr.Keys.generate();
    final session = _Session();
    final container = ProviderContainer(
      overrides: [
        savedPrefsProvider.overrideWithValue(
          await SharedPreferences.getInstance(),
        ),
        relayConfigProvider.overrideWith(() => _Config('  ${keys.nsec} \n')),
        relaySessionProvider.overrideWith(() => session),
      ],
    );
    addTearDown(container.dispose);
    final listener = container.listen(communityThemeProvider, (_, _) {});
    addTearDown(listener.close);
    session.transition(SessionStatus.connected);
    await container.pump();
    final crypto = container.read(communityThemeCryptoProvider)!;
    expect(crypto.decrypt(crypto.encrypt('theme')), 'theme');
    await session.firstPublished.future.timeout(const Duration(seconds: 5));
    expect(session.published, isNotEmpty);
    for (final event in session.published) {
      expect(event.pubkey, keys.public);
      expect(nostr.Event.fromMap(event.toJson()).isValid(), isTrue);
    }
  });

  test(
    'reconnect reuses crypto but a community or key change replaces it',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final keys = nostr.Keys.generate();
      final observer = _CryptoObserver();
      final session = _Session();
      final container = ProviderContainer(
        observers: [observer],
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          relayConfigProvider.overrideWith(() => _Config(keys.nsec)),
          relaySessionProvider.overrideWith(() => session),
          activeCommunityProvider.overrideWith((ref) async => null),
          appLifecycleProvider.overrideWith(_Lifecycle.new),
        ],
      );
      addTearDown(container.dispose);
      await container.read(activeCommunityProvider.future);
      final listeners = [
        container.listen(channelStarsProvider, (_, _) {}),
        container.listen(channelMutesProvider, (_, _) {}),
        container.listen(channelSectionsProvider, (_, _) {}),
        container.listen(channelSortProvider, (_, _) {}),
        container.listen(readStateProvider, (_, _) {}),
        container.listen(communityThemeProvider, (_, _) {}),
      ];
      addTearDown(() {
        for (final listener in listeners) {
          listener.close();
        }
      });
      await container.pump();
      session.transition(SessionStatus.connected);
      await container.pump();
      expect(observer.creations, 6);
      final initial = _cryptoProviders.map(container.read).toList();
      final creations = observer.creations;
      expect(creations, 6);

      for (final status in [
        SessionStatus.connected,
        SessionStatus.disconnected,
        SessionStatus.connecting,
        SessionStatus.connected,
      ]) {
        session.transition(status);
        await container.pump();
        for (var i = 0; i < initial.length; i++) {
          expect(container.read(_cryptoProviders[i]), same(initial[i]));
        }
      }
      expect(observer.creations, creations);
      final first = container.read(channelStarsCryptoProvider)!;
      final ciphertext = first.encrypt('retained preference');
      expect(
        container.read(channelStarsCryptoProvider)!.decrypt(ciphertext),
        'retained preference',
      );

      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://other.example', nsec: keys.nsec);
      await container.pump();
      for (var i = 0; i < initial.length; i++) {
        expect(container.read(_cryptoProviders[i]), isNot(same(initial[i])));
      }
      // Same identity still decrypts its protocol data after a community switch.
      expect(
        container.read(channelStarsCryptoProvider)!.decrypt(ciphertext),
        'retained preference',
      );

      container
          .read(relayConfigProvider.notifier)
          .update(
            baseUrl: 'https://other.example',
            nsec: nostr.Keys.generate().nsec,
          );
      await container.pump();
      expect(
        () => container.read(channelStarsCryptoProvider)!.decrypt(ciphertext),
        throwsA(anything),
      );
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://other.example');
      await container.pump();
      for (final provider in _cryptoProviders) {
        expect(container.read(provider), isNull);
      }
    },
  );
}

final _cryptoProviders = <Provider<Object?>>[
  channelStarsCryptoProvider,
  channelMutesCryptoProvider,
  channelSectionsCryptoProvider,
  channelSortCryptoProvider,
  readStateCryptoProvider,
  communityThemeCryptoProvider,
];

final class _CryptoObserver extends ProviderObserver {
  int creations = 0;
  @override
  void didAddProvider(ProviderObserverContext context, Object? value) {
    if (_cryptoProviders.contains(context.provider)) creations++;
  }

  @override
  void didUpdateProvider(
    ProviderObserverContext context,
    Object? previousValue,
    Object? newValue,
  ) {
    if (_cryptoProviders.contains(context.provider)) creations++;
  }
}

class _Config extends RelayConfigNotifier {
  _Config(this.nsec);
  final String nsec;
  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: nsec);
}

class _Session extends RelaySessionNotifier {
  final published = <NostrEvent>[];
  final firstPublished = Completer<void>();
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);
  void transition(SessionStatus status) => state = SessionState(status: status);
  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [];
  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String)? onClosed,
  }) async => () {};
  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published.add(event);
    if (!firstPublished.isCompleted) firstPublished.complete();
    return event;
  }
}

class _Lifecycle extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}
