import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  test('profile updates preserve existing kind:0 metadata', () async {
    final keys = nostr.Keys.generate();
    final relaySession = _ProfileRelaySession(
      NostrEvent(
        id: 'profile-1',
        pubkey: keys.public,
        createdAt: 1,
        kind: EventKind.profile,
        tags: const [],
        content: jsonEncode({
          'name': 'alice',
          'display_name': 'Alice',
          'about': 'Building Buzz',
          'picture': 'https://relay.example/alice.png',
          'nip05': 'alice@example.com',
          'custom': 'preserve-me',
        }),
        sig: 'sig',
      ),
    );
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(
          () => _FixedRelayConfigNotifier(keys.nsec),
        ),
        relaySessionProvider.overrideWith(() => relaySession),
      ],
    );
    addTearDown(container.dispose);

    await container.read(profileProvider.future);
    await container.read(profileProvider.notifier).updateDisplayName('Alice L');

    expect(relaySession.published, hasLength(1));
    final content =
        jsonDecode(relaySession.published.single.content)
            as Map<String, dynamic>;
    expect(content['display_name'], 'Alice L');
    expect(content['about'], 'Building Buzz');
    expect(content['picture'], 'https://relay.example/alice.png');
    expect(content['nip05'], 'alice@example.com');
    expect(content['custom'], 'preserve-me');
    expect(
      container.read(profileProvider).requireValue?.displayName,
      'Alice L',
    );
  });

  test('profile updates fail closed while hydration is pending', () async {
    final keys = nostr.Keys.generate();
    final history = Completer<List<NostrEvent>>();
    final relaySession = _ControlledProfileRelaySession(
      fetch: () => history.future,
    );
    final container = _profileContainer(keys.nsec, relaySession);
    addTearDown(container.dispose);

    container.read(profileProvider);
    await expectLater(
      container.read(profileProvider.notifier).updateDisplayName('Unsafe'),
      throwsStateError,
    );
    expect(relaySession.published, isEmpty);
    history.complete(const []);
  });

  test('profile updates fail closed after hydration errors', () async {
    final keys = nostr.Keys.generate();
    final relaySession = _ControlledProfileRelaySession(
      fetch: () async => throw Exception('relay unavailable'),
    );
    final container = _profileContainer(keys.nsec, relaySession);
    addTearDown(container.dispose);

    container.read(profileProvider);
    await Future<void>.delayed(Duration.zero);
    expect(container.read(profileProvider).hasError, isTrue);
    await expectLater(
      container.read(profileProvider.notifier).updateAbout('Unsafe'),
      throwsStateError,
    );
    expect(relaySession.published, isEmpty);
  });

  test('a confirmed empty history can publish a new profile', () async {
    final keys = nostr.Keys.generate();
    final relaySession = _ControlledProfileRelaySession(fetch: () async => []);
    final container = _profileContainer(keys.nsec, relaySession);
    addTearDown(container.dispose);

    expect(await container.read(profileProvider.future), isNull);
    await container.read(profileProvider.notifier).updateDisplayName('Alice');

    expect(relaySession.published, hasLength(1));
    expect(jsonDecode(relaySession.published.single.content), {
      'display_name': 'Alice',
    });
  });

  test('same-second profile replacements use monotonic timestamps', () async {
    final keys = nostr.Keys.generate();
    final futureTimestamp = DateTime.now().millisecondsSinceEpoch ~/ 1000 + 60;
    final relaySession = _ControlledProfileRelaySession(
      fetch: () async => [
        NostrEvent(
          id: 'profile-future',
          pubkey: keys.public,
          createdAt: futureTimestamp,
          kind: EventKind.profile,
          tags: const [],
          content: '{}',
          sig: 'sig',
        ),
      ],
    );
    final container = _profileContainer(keys.nsec, relaySession);
    addTearDown(container.dispose);

    await container.read(profileProvider.future);
    await container.read(profileProvider.notifier).updateDisplayName('Alice');
    await container.read(profileProvider.notifier).updateAbout('Hello');

    expect(relaySession.published.map((event) => event.createdAt), [
      futureTimestamp + 1,
      futureTimestamp + 2,
    ]);
  });

  test(
    'profile updates merge the current relay head before publishing',
    () async {
      final keys = nostr.Keys.generate();
      var history = [
        NostrEvent(
          id: 'profile-initial',
          pubkey: keys.public,
          createdAt: 10,
          kind: EventKind.profile,
          tags: const [],
          content: jsonEncode({
            'display_name': 'Initial',
            'about': 'Initial about',
            'custom': 'initial',
          }),
          sig: 'sig',
        ),
      ];
      final relaySession = _ControlledProfileRelaySession(
        fetch: () async => history,
      );
      final container = _profileContainer(keys.nsec, relaySession);
      addTearDown(container.dispose);

      await container.read(profileProvider.future);
      history = [
        NostrEvent(
          id: 'profile-remote',
          pubkey: keys.public,
          createdAt: 20,
          kind: EventKind.profile,
          tags: const [],
          content: jsonEncode({
            'display_name': 'Remote',
            'about': 'Remote about',
            'custom': 'remote',
          }),
          sig: 'sig',
        ),
      ];

      await container
          .read(profileProvider.notifier)
          .updateDisplayName('Mobile');

      final content =
          jsonDecode(relaySession.published.single.content)
              as Map<String, dynamic>;
      expect(content, {
        'display_name': 'Mobile',
        'about': 'Remote about',
        'custom': 'remote',
      });
      expect(relaySession.published.single.createdAt, greaterThan(20));
    },
  );

  test(
    'a competing profile head does not become optimistic local state',
    () async {
      final keys = nostr.Keys.generate();
      final relaySession = _LosingProfileRelaySession(
        NostrEvent(
          id: 'profile-initial',
          pubkey: keys.public,
          createdAt: 10,
          kind: EventKind.profile,
          tags: const [],
          content: jsonEncode({'display_name': 'Initial'}),
          sig: 'sig',
        ),
      );
      final container = _profileContainer(keys.nsec, relaySession);
      addTearDown(container.dispose);

      await container.read(profileProvider.future);

      await expectLater(
        container.read(profileProvider.notifier).updateDisplayName('Mobile'),
        throwsStateError,
      );
      expect(relaySession.published, hasLength(1));
      expect(
        container.read(profileProvider).requireValue?.displayName,
        'Initial',
      );
    },
  );

  test(
    'overlapping profile updates serialize their full merge cycles',
    () async {
      final keys = nostr.Keys.generate();
      final relaySession = _ControlledProfileRelaySession(
        fetch: () async => [
          NostrEvent(
            id: 'profile-initial',
            pubkey: keys.public,
            createdAt: 10,
            kind: EventKind.profile,
            tags: const [],
            content: jsonEncode({
              'display_name': 'Initial',
              'about': 'Initial about',
            }),
            sig: 'sig',
          ),
        ],
      );
      final container = _profileContainer(keys.nsec, relaySession);
      addTearDown(container.dispose);

      await container.read(profileProvider.future);
      await Future.wait([
        container.read(profileProvider.notifier).updateDisplayName('Mobile'),
        container.read(profileProvider.notifier).updateAbout('Mobile about'),
      ]);

      expect(relaySession.published, hasLength(2));
      expect(jsonDecode(relaySession.published.last.content), {
        'display_name': 'Mobile',
        'about': 'Mobile about',
      });
      expect(
        relaySession.published.last.createdAt,
        greaterThan(relaySession.published.first.createdAt),
      );
    },
  );

  test('profile updates abort when the active community changes', () async {
    final keys = nostr.Keys.generate();
    final otherKeys = nostr.Keys.generate();
    final initial = NostrEvent(
      id: 'profile-initial',
      pubkey: keys.public,
      createdAt: 10,
      kind: EventKind.profile,
      tags: const [],
      content: jsonEncode({'display_name': 'Initial'}),
      sig: 'sig',
    );
    final patchFetchStarted = Completer<void>();
    final patchHistory = Completer<List<NostrEvent>>();
    var fetchCount = 0;
    final relaySession = _ControlledProfileRelaySession(
      fetch: () async {
        fetchCount += 1;
        if (fetchCount == 1) return [initial];
        if (!patchFetchStarted.isCompleted) patchFetchStarted.complete();
        return patchHistory.future;
      },
    );
    final config = _MutableRelayConfigNotifier(keys.nsec);
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(() => config),
        relaySessionProvider.overrideWith(() => relaySession),
      ],
    );
    addTearDown(container.dispose);

    await container.read(profileProvider.future);
    final update = container
        .read(profileProvider.notifier)
        .updateDisplayName('Mobile');
    await patchFetchStarted.future;
    config.update(baseUrl: 'https://other-relay.example', nsec: otherKeys.nsec);
    patchHistory.complete([initial]);

    await expectLater(update, throwsStateError);
    expect(relaySession.published, isEmpty);
  });

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

class _FixedRelayConfigNotifier extends RelayConfigNotifier {
  _FixedRelayConfigNotifier(this.nsec);

  final String nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: nsec);
}

class _MutableRelayConfigNotifier extends RelayConfigNotifier {
  _MutableRelayConfigNotifier(this.initialNsec);

  final String initialNsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: initialNsec);
}

class _ProfileRelaySession extends RelaySessionNotifier {
  _ProfileRelaySession(this.profile);

  final NostrEvent profile;
  final List<NostrEvent> published = [];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [profile, ...published];

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published.add(event);
    return event;
  }
}

ProviderContainer _profileContainer(
  String nsec,
  RelaySessionNotifier relaySession,
) => ProviderContainer(
  overrides: [
    relayConfigProvider.overrideWith(() => _FixedRelayConfigNotifier(nsec)),
    relaySessionProvider.overrideWith(() => relaySession),
  ],
);

class _ControlledProfileRelaySession extends RelaySessionNotifier {
  _ControlledProfileRelaySession({required this.fetch});

  final Future<List<NostrEvent>> Function() fetch;
  final List<NostrEvent> published = [];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [...await fetch(), ...published];

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published.add(event);
    return event;
  }
}

class _LosingProfileRelaySession extends RelaySessionNotifier {
  _LosingProfileRelaySession(this.initial);

  final NostrEvent initial;
  final List<NostrEvent> published = [];
  NostrEvent? competing;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [competing ?? initial];

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published.add(event);
    competing = NostrEvent(
      id: 'profile-competing',
      pubkey: initial.pubkey,
      createdAt: event.createdAt + 1,
      kind: EventKind.profile,
      tags: const [],
      content: jsonEncode({'display_name': 'Remote'}),
      sig: 'sig',
    );
    return event;
  }
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
