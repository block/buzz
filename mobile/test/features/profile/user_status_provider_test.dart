import 'package:buzz/features/profile/user_status_cache_provider.dart';
import 'package:buzz/features/profile/user_status.dart';
import 'package:buzz/features/profile/user_status_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  test(
    'publishes the selected status expiration on the final signed event',
    () async {
      final keys = nostr.Keys.generate();
      final relaySession = _RecordingRelaySession();
      final expiresAt = DateTime.fromMillisecondsSinceEpoch(
        1_900_000_000 * 1000,
      );
      final container = ProviderContainer(
        overrides: [
          relayConfigProvider.overrideWith(
            () => _FixedRelayConfigNotifier(keys.nsec),
          ),
          relaySessionProvider.overrideWith(() => relaySession),
          userStatusCacheProvider.overrideWith(_EmptyUserStatusCache.new),
        ],
      );
      addTearDown(container.dispose);

      await container.read(userStatusProvider.future);
      await container
          .read(userStatusProvider.notifier)
          .setStatus(' Focusing ', '\u{1F3AF}', expiresAt: expiresAt);

      final event = relaySession.published.single;
      expect(event.kind, EventKind.userStatus);
      expect(event.content, 'Focusing');
      expect(event.tags, contains(equals(['d', 'general'])));
      expect(event.tags, contains(equals(['emoji', '\u{1F3AF}'])));
      expect(event.tags, contains(equals(['expiration', '1900000000'])));
    },
  );

  testWidgets('clears the current status and shared cache at expiration', (
    tester,
  ) async {
    final keys = nostr.Keys.generate();
    final relaySession = _RecordingRelaySession();
    final statusCache = _RecordingUserStatusCache();
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(
          () => _FixedRelayConfigNotifier(keys.nsec),
        ),
        relaySessionProvider.overrideWith(() => relaySession),
        userStatusCacheProvider.overrideWith(() => statusCache),
      ],
    );
    addTearDown(container.dispose);

    await container.read(userStatusProvider.future);
    await container
        .read(userStatusProvider.notifier)
        .setStatus('Focusing', '🎯', expiresAt: DateTime.now());

    expect(container.read(userStatusProvider).value, isNotNull);
    expect(statusCache.updates.last.$2, isNotNull);

    await tester.pump(const Duration(milliseconds: 1));

    expect(container.read(userStatusProvider).value, isNull);
    expect(statusCache.updates.last, (keys.public.toLowerCase(), null));
  });

  testWidgets('clears another user status without a new relay event', (
    tester,
  ) async {
    final container = ProviderContainer(
      overrides: [
        relayClientProvider.overrideWithValue(
          RelayClient(baseUrl: 'https://relay.example'),
        ),
        relaySessionProvider.overrideWith(_DisconnectedRelaySession.new),
      ],
    );
    addTearDown(container.dispose);

    final cache = container.read(userStatusCacheProvider.notifier);
    cache.updateStatus(
      'alice',
      UserStatus(
        text: 'Focusing',
        emoji: '',
        updatedAt: 1,
        expiresAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
      ),
    );
    expect(container.read(userStatusCacheProvider)['alice'], isNotNull);

    await tester.pump(const Duration(milliseconds: 1));

    expect(container.read(userStatusCacheProvider)['alice'], isNull);
  });

  test('ignores an out-of-range expiration in the shared cache scheduler', () {
    final container = ProviderContainer(
      overrides: [
        relayClientProvider.overrideWithValue(
          RelayClient(baseUrl: 'https://relay.example'),
        ),
        relaySessionProvider.overrideWith(_DisconnectedRelaySession.new),
      ],
    );
    addTearDown(container.dispose);

    final status = UserStatus(
      text: 'Focusing',
      emoji: '',
      updatedAt: 1,
      expiresAt: 9_000_000_000_000,
    );
    expect(status.expirationDateTime, isNull);

    expect(
      () => container
          .read(userStatusCacheProvider.notifier)
          .updateStatus('alice', status),
      returnsNormally,
    );
    expect(container.read(userStatusCacheProvider)['alice'], same(status));
  });

  test(
    'ignores an out-of-range expiration in the current-user scheduler',
    () async {
      final keys = nostr.Keys.generate();
      final relaySession = _StatusRelaySession(
        NostrEvent(
          id: 'status-1',
          pubkey: keys.public,
          createdAt: 1,
          kind: EventKind.userStatus,
          tags: const [
            ['d', 'general'],
            ['expiration', '9000000000000'],
          ],
          content: 'Focusing',
          sig: 'sig',
        ),
      );
      final container = ProviderContainer(
        overrides: [
          relayConfigProvider.overrideWith(
            () => _FixedRelayConfigNotifier(keys.nsec),
          ),
          relaySessionProvider.overrideWith(() => relaySession),
          userStatusCacheProvider.overrideWith(_EmptyUserStatusCache.new),
        ],
      );
      addTearDown(container.dispose);

      final status = await container.read(userStatusProvider.future);

      expect(status?.text, 'Focusing');
      expect(status?.expiresAt, 9_000_000_000_000);
      expect(status?.expirationDateTime, isNull);
    },
  );
}

class _FixedRelayConfigNotifier extends RelayConfigNotifier {
  _FixedRelayConfigNotifier(this.nsec);

  final String nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: nsec);
}

class _RecordingRelaySession extends RelaySessionNotifier {
  final List<NostrEvent> published = [];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [];

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published.add(event);
    return event;
  }
}

class _StatusRelaySession extends _RecordingRelaySession {
  _StatusRelaySession(this.status);

  final NostrEvent status;

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [status];
}

class _DisconnectedRelaySession extends RelaySessionNotifier {
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);
}

class _RecordingUserStatusCache extends UserStatusCacheNotifier {
  final List<(String, UserStatus?)> updates = [];

  @override
  Map<String, UserStatus?> build() => {};

  @override
  void updateStatus(String pubkey, UserStatus? status) {
    updates.add((pubkey, status));
  }
}

class _EmptyUserStatusCache extends UserStatusCacheNotifier {
  @override
  Map<String, UserStatus?> build() => {};
}
