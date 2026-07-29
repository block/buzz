import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/user_cache_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:pointycastle/digests/sha256.dart';

void main() {
  test('updates the display name without discarding kind-0 metadata', () async {
    final keys = nostr.Keys.generate();
    final owner = nostr.Keys.generate();
    final nsec = keys.nsec;
    final authTag = _authTag(owner, keys.public);
    final session = _RecordingRelaySession(keys.public, authTag);
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(() => _FixedRelayConfigNotifier(nsec)),
        myPubkeyProvider.overrideWithValue(keys.public),
        relaySessionProvider.overrideWith(() => session),
      ],
    );
    addTearDown(container.dispose);

    await container.read(profileProvider.future);
    await container
        .read(profileProvider.notifier)
        .updateDisplayName('  Daniel V  ');

    final published = session.published.single;
    final metadata = jsonDecode(published.content) as Map<String, dynamic>;

    expect(published.kind, 0);
    expect(published.createdAt, 4102444801);
    expect(published.tags, [authTag]);
    expect(metadata['display_name'], 'Daniel V');
    expect(metadata['name'], 'daniel');
    expect(metadata['picture'], 'https://example.com/avatar.png');
    expect(metadata['about'], 'Builder');
    expect(metadata['nip05'], 'daniel@example.com');
    expect(metadata['custom_field'], 'keep-me');
    expect(
      container.read(profileProvider).requireValue?.displayName,
      'Daniel V',
    );
    expect(
      container.read(profileProvider).requireValue?.ownerPubkey,
      owner.public,
    );
    expect(
      container.read(userCacheProvider)[keys.public]?.displayName,
      'Daniel V',
    );
    expect(
      container.read(userCacheProvider)[keys.public]?.ownerPubkey,
      owner.public,
    );
  });

  test('rejects display names longer than the profile storage limit', () async {
    final keys = nostr.Keys.generate();
    final owner = nostr.Keys.generate();
    final session = _RecordingRelaySession(
      keys.public,
      _authTag(owner, keys.public),
    );
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(
          () => _FixedRelayConfigNotifier(keys.nsec),
        ),
        myPubkeyProvider.overrideWithValue(keys.public),
        relaySessionProvider.overrideWith(() => session),
      ],
    );
    addTearDown(container.dispose);

    await container.read(profileProvider.future);

    await expectLater(
      container.read(profileProvider.notifier).updateDisplayName('x' * 256),
      throwsArgumentError,
    );
    expect(session.published, isEmpty);
  });

  test('does not update local state when another kind-0 event wins', () async {
    final keys = nostr.Keys.generate();
    final owner = nostr.Keys.generate();
    final session = _RecordingRelaySession(
      keys.public,
      _authTag(owner, keys.public),
      confirmPublished: false,
    );
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(
          () => _FixedRelayConfigNotifier(keys.nsec),
        ),
        myPubkeyProvider.overrideWithValue(keys.public),
        relaySessionProvider.overrideWith(() => session),
      ],
    );
    addTearDown(container.dispose);

    await container.read(profileProvider.future);

    await expectLater(
      container
          .read(profileProvider.notifier)
          .updateDisplayName('Superseded name'),
      throwsStateError,
    );
    expect(container.read(profileProvider).requireValue?.displayName, 'Daniel');
  });

  test('local update wins over an older in-flight cache fetch', () async {
    final keys = nostr.Keys.generate();
    final session = _PendingRelaySession();
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(
          () => _FixedRelayConfigNotifier(keys.nsec),
        ),
        relaySessionProvider.overrideWith(() => session),
      ],
    );
    addTearDown(container.dispose);

    container.read(userCacheProvider.notifier).get(keys.public);
    await Future<void>.delayed(const Duration(milliseconds: 75));

    container
        .read(userCacheProvider.notifier)
        .updateProfile(
          UserProfile(pubkey: keys.public, displayName: 'New name'),
        );
    session.history.complete([
      NostrEvent(
        id: 'stale-profile',
        pubkey: keys.public,
        createdAt: 1,
        kind: 0,
        tags: const [],
        content: jsonEncode({'display_name': 'Old name'}),
        sig: 'sig',
      ),
    ]);
    await Future<void>.delayed(Duration.zero);

    expect(
      container.read(userCacheProvider)[keys.public]?.displayName,
      'New name',
    );
  });

  test('uses ownership from the latest profile event only', () async {
    final keys = nostr.Keys.generate();
    final owner = nostr.Keys.generate();
    final session = _RecordingRelaySession(
      keys.public,
      _authTag(owner, keys.public),
      removeAuthBeforeUpdate: true,
    );
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(
          () => _FixedRelayConfigNotifier(keys.nsec),
        ),
        myPubkeyProvider.overrideWithValue(keys.public),
        relaySessionProvider.overrideWith(() => session),
      ],
    );
    addTearDown(container.dispose);

    expect(
      (await container.read(profileProvider.future))?.ownerPubkey,
      owner.public,
    );
    await container
        .read(profileProvider.notifier)
        .updateDisplayName('Daniel V');

    expect(container.read(profileProvider).requireValue?.ownerPubkey, isNull);
    expect(container.read(userCacheProvider)[keys.public]?.ownerPubkey, isNull);
  });

  test('repairs non-object kind-0 metadata with a display name', () async {
    final keys = nostr.Keys.generate();
    final owner = nostr.Keys.generate();
    final session = _RecordingRelaySession(
      keys.public,
      _authTag(owner, keys.public),
      initialContent: '[]',
    );
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(
          () => _FixedRelayConfigNotifier(keys.nsec),
        ),
        myPubkeyProvider.overrideWithValue(keys.public),
        relaySessionProvider.overrideWith(() => session),
      ],
    );
    addTearDown(container.dispose);

    await container.read(profileProvider.future);
    await container
        .read(profileProvider.notifier)
        .updateDisplayName('Repaired');

    expect(jsonDecode(session.published.single.content), {
      'display_name': 'Repaired',
    });
  });

  test('aborts when the active relay changes during the update', () async {
    final initialKeys = nostr.Keys.generate();
    final nextKeys = nostr.Keys.generate();
    final session = _CommunitySwitchRelaySession(initialKeys.public);
    final container = ProviderContainer(
      overrides: [
        relayConfigProvider.overrideWith(
          () => _FixedRelayConfigNotifier(initialKeys.nsec),
        ),
        relaySessionProvider.overrideWith(() => session),
      ],
    );
    addTearDown(container.dispose);

    await container.read(profileProvider.future);
    final update = container
        .read(profileProvider.notifier)
        .updateDisplayName('Wrong community');
    await Future<void>.delayed(Duration.zero);

    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://other-relay.example', nsec: nextKeys.nsec);
    session.pendingUpdate.complete([session.existingProfile]);

    await expectLater(update, throwsStateError);
    expect(session.published, isEmpty);
  });
}

class _FixedRelayConfigNotifier extends RelayConfigNotifier {
  _FixedRelayConfigNotifier(this._nsec);

  final String _nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: _nsec);
}

class _RecordingRelaySession extends RelaySessionNotifier {
  _RecordingRelaySession(
    this.pubkey,
    this.authTag, {
    this.confirmPublished = true,
    this.removeAuthBeforeUpdate = false,
    this.initialContent,
  });

  final String pubkey;
  final List<String> authTag;
  final bool confirmPublished;
  final bool removeAuthBeforeUpdate;
  final String? initialContent;
  final List<NostrEvent> published = [];
  int _fetchCount = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (confirmPublished && published.isNotEmpty) return [published.last];

    _fetchCount += 1;
    return [
      NostrEvent(
        id: 'existing-profile',
        pubkey: pubkey,
        createdAt: 4102444800,
        kind: 0,
        tags: removeAuthBeforeUpdate && _fetchCount > 1 ? const [] : [authTag],
        content:
            initialContent ??
            jsonEncode({
              'display_name': 'Daniel',
              'name': 'daniel',
              'picture': 'https://example.com/avatar.png',
              'about': 'Builder',
              'nip05': 'daniel@example.com',
              'custom_field': 'keep-me',
            }),
        sig: 'sig',
      ),
    ];
  }

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published.add(event);
    return event;
  }
}

class _PendingRelaySession extends RelaySessionNotifier {
  final history = Completer<List<NostrEvent>>();

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) => history.future;
}

class _CommunitySwitchRelaySession extends RelaySessionNotifier {
  _CommunitySwitchRelaySession(this.pubkey);

  final String pubkey;
  final pendingUpdate = Completer<List<NostrEvent>>();
  final List<NostrEvent> published = [];
  int _fetchCount = 0;

  NostrEvent get existingProfile => NostrEvent(
    id: 'existing-profile',
    pubkey: pubkey,
    createdAt: 1,
    kind: 0,
    tags: const [],
    content: jsonEncode({'display_name': 'Daniel'}),
    sig: 'sig',
  );

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    _fetchCount += 1;
    return _fetchCount == 2
        ? pendingUpdate.future
        : Future.value([existingProfile]);
  }

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published.add(event);
    return event;
  }
}

List<String> _authTag(nostr.Keys owner, String agentPubkey) {
  final preimage = utf8.encode('nostr:agent-auth:$agentPubkey:');
  final digest = SHA256Digest().process(Uint8List.fromList(preimage));
  final message = digest
      .map((byte) => byte.toRadixString(16).padLeft(2, '0'))
      .join();
  final signature = nostr.Schnorr.sign(
    secretKey: owner.secret,
    message: message,
  );
  return ['auth', owner.public, '', signature];
}
