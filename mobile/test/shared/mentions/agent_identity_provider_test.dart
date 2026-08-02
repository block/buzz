import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/channels/agent_activity/working_bots_provider.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  test('filters archived identities from the active agent directory', () {
    final active = _agentProfileEvent(
      pubkey: _agentPubkey,
      displayName: 'Active agent',
    );
    final archived = _agentProfileEvent(
      pubkey: _archivedAgentPubkey,
      displayName: 'Retired agent',
    );

    expect(
      activeAgentDirectoryEntries(
        [active, archived],
        archivedPubkeys: const {_archivedAgentPubkey},
      ).map((entry) => entry.pubkey),
      [_agentPubkey],
    );
  });

  test('uses the newest valid relay archive snapshot', () {
    final older = _archiveSnapshot(
      id: 'b',
      createdAt: 10,
      pubkeys: const [_agentPubkey],
    );
    final newer = _archiveSnapshot(
      id: 'c',
      createdAt: 20,
      pubkeys: const [_archivedAgentPubkey],
    );
    final forged = _archiveSnapshot(
      id: 'a',
      createdAt: 30,
      pubkeys: const [_agentPubkey],
      pubkey: 'forged-relay',
    );

    expect(
      archivedIdentityPubkeysFromSnapshots(
        [older, newer, forged],
        relayPubkey: _relayPubkey,
        verifySignature: (_) => true,
      ),
      const {_archivedAgentPubkey},
    );
  });

  test('rejects missing, malformed, and duplicate NIP-70 tags', () {
    for (final protectionTags in <List<List<String>>>[
      const [],
      const [
        ['-', 'extra'],
      ],
      const [
        ['-'],
        ['-'],
      ],
    ]) {
      final snapshot = _archiveSnapshot(
        id: 'invalid',
        createdAt: 10,
        pubkeys: const [_archivedAgentPubkey],
        protectionTags: protectionTags,
      );

      expect(
        archivedIdentityPubkeysFromSnapshots(
          [snapshot],
          relayPubkey: _relayPubkey,
          verifySignature: (_) => true,
        ),
        isEmpty,
      );
    }
  });

  test('a live relay archive snapshot removes an active identity', () async {
    final relayKeys = nostr.Keys.generate();
    final relaySession = _ArchiveRelaySessionNotifier(
      _signedArchiveSnapshot(relayKeys, const []),
    );
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(() => relaySession),
        relayIdentityPubkeyProvider.overrideWith(
          (ref) async => relayKeys.public,
        ),
      ],
    );
    addTearDown(container.dispose);
    final keepAlive = container.listen(
      archivedIdentityPubkeysProvider,
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(keepAlive.close);

    expect(
      await container.read(archivedIdentityPubkeysProvider.future),
      isEmpty,
    );
    await relaySession.subscribed;

    final archivedSnapshot = _signedArchiveSnapshot(relayKeys, const [
      _archivedAgentPubkey,
    ]);
    relaySession.snapshot = archivedSnapshot;
    relaySession.emit(archivedSnapshot);
    await _pumpEventQueue();

    expect(await container.read(archivedIdentityPubkeysProvider.future), {
      _archivedAgentPubkey,
    });
  });

  test('refreshes channel bot roles from live membership updates', () async {
    final relaySession = _MembershipRelaySessionNotifier([
      _membershipEvent(role: 'bot'),
      _membershipEvent(role: 'member'),
    ]);
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => relaySession)],
    );
    addTearDown(container.dispose);
    final keepAlive = container.listen(
      channelBotPubkeysProvider(_channelId),
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(keepAlive.close);

    expect(await container.read(channelBotPubkeysProvider(_channelId).future), {
      _agentPubkey,
    });
    await relaySession.subscribed;
    expect(relaySession.liveFilters.single.kinds, const [39002]);
    expect(relaySession.liveFilters.single.tags['#h'], [_channelId]);

    relaySession.emit(_membershipEvent(role: 'member'));
    await _pumpEventQueue();

    expect(
      await container.read(channelBotPubkeysProvider(_channelId).future),
      isEmpty,
    );
  });

  test('refreshes channel members from live membership updates', () async {
    final relaySession = _MembershipRelaySessionNotifier([
      _membershipEvent(role: 'bot'),
      _membershipEvent(role: 'member'),
    ]);
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => relaySession)],
    );
    addTearDown(container.dispose);
    final keepAlive = container.listen(
      channelMembersProvider(_channelId),
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(keepAlive.close);

    expect(
      (await container.read(
        channelMembersProvider(_channelId).future,
      )).single.role,
      'bot',
    );
    await relaySession.subscribed;

    relaySession.emit(_membershipEvent(role: 'member'));
    await _pumpEventQueue();

    expect(
      (await container.read(
        channelMembersProvider(_channelId).future,
      )).single.role,
      'member',
    );
  });

  test('disposes the live role subscription without consumers', () async {
    final relaySession = _MembershipRelaySessionNotifier([
      _membershipEvent(role: 'bot'),
    ]);
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => relaySession)],
    );
    addTearDown(container.dispose);
    final keepAlive = container.listen(
      channelBotPubkeysProvider(_channelId),
      (_, _) {},
      fireImmediately: true,
    );

    await container.read(channelBotPubkeysProvider(_channelId).future);
    await relaySession.subscribed;
    keepAlive.close();
    await container.pump();

    expect(relaySession.unsubscribeCount, 1);
  });

  test(
    'does not retain a live role subscription through working bots',
    () async {
      final relaySession = _MembershipRelaySessionNotifier([
        _membershipEvent(role: 'bot'),
      ]);
      final container = ProviderContainer(
        overrides: [relaySessionProvider.overrideWith(() => relaySession)],
      );
      addTearDown(container.dispose);
      final keepAlive = container.listen(
        workingBotPubkeysProvider(_channelId),
        (_, _) {},
        fireImmediately: true,
      );

      await relaySession.subscribed;
      keepAlive.close();
      await container.pump();

      expect(relaySession.unsubscribeCount, 1);
    },
  );

  test('blank profile labels defer to the directory label', () {
    const pubkey = 'deadbeef0123456789';

    expect(
      mentionNamesWithDirectoryLabels(
        mentionPubkeys: const [pubkey],
        profileMentionNames: const {pubkey: '  '},
        directoryDisplayNames: const {pubkey: 'Directory bot'},
        agentMentionPubkeys: const {pubkey},
      ),
      const {pubkey: 'Directory bot'},
    );
  });
}

const _channelId = '11111111-1111-4111-8111-111111111111';
const _agentPubkey =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _archivedAgentPubkey =
    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
const _relayPubkey =
    'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc';

NostrEvent _agentProfileEvent({
  required String pubkey,
  required String displayName,
}) => NostrEvent(
  id: 'profile-$pubkey',
  pubkey: pubkey,
  createdAt: 1,
  kind: 10100,
  tags: const [],
  content: '{"display_name":"$displayName"}',
  sig: 'sig',
);

NostrEvent _archiveSnapshot({
  required String id,
  required int createdAt,
  required List<String> pubkeys,
  String pubkey = _relayPubkey,
  List<List<String>> protectionTags = const [
    ['-'],
  ],
}) => NostrEvent(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  kind: 13535,
  tags: [
    ...protectionTags,
    for (final archivedPubkey in pubkeys) ['p', archivedPubkey],
  ],
  content: '',
  sig: 'sig',
);

NostrEvent _signedArchiveSnapshot(nostr.Keys keys, List<String> pubkeys) =>
    NostrEvent.fromJson(
      nostr.Event.from(
        kind: 13535,
        content: '',
        tags: [
          ['-'],
          for (final pubkey in pubkeys) ['p', pubkey],
        ],
        secretKey: keys.secret,
        verify: false,
      ).toMap(),
    );

NostrEvent _membershipEvent({required String role}) => NostrEvent(
  id: 'membership-$role',
  pubkey: 'owner',
  createdAt: 1,
  kind: 39002,
  tags: [
    ['d', _channelId],
    ['h', _channelId],
    ['p', _agentPubkey, 'wss://relay.example', role],
  ],
  content: '',
  sig: 'sig',
);

Future<void> _pumpEventQueue() async {
  await Future<void>.delayed(Duration.zero);
  await Future<void>.delayed(Duration.zero);
}

class _MembershipRelaySessionNotifier extends RelaySessionNotifier {
  final List<NostrEvent> _memberships;
  final List<NostrFilter> liveFilters = [];
  final List<_LiveSubscription> _subscriptions = [];
  final Completer<void> _subscribed = Completer<void>();
  var unsubscribeCount = 0;
  var _membershipIndex = 0;

  _MembershipRelaySessionNotifier(this._memberships);

  Future<void> get subscribed => _subscribed.future;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    return [_memberships[_membershipIndex++]];
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    liveFilters.add(filter);
    final subscription = _LiveSubscription(filter, onEvent);
    _subscriptions.add(subscription);
    if (!_subscribed.isCompleted) _subscribed.complete();
    return () {
      unsubscribeCount++;
      _subscriptions.remove(subscription);
    };
  }

  void emit(NostrEvent event) {
    for (final subscription in List.of(_subscriptions)) {
      if (_matches(subscription.filter, event)) {
        subscription.onEvent(event);
      }
    }
  }
}

class _ArchiveRelaySessionNotifier extends RelaySessionNotifier {
  NostrEvent snapshot;
  final Completer<void> _subscribed = Completer<void>();
  final List<void Function(NostrEvent)> _listeners = [];

  _ArchiveRelaySessionNotifier(this.snapshot);

  Future<void> get subscribed => _subscribed.future;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [snapshot];

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    _listeners.add(onEvent);
    if (!_subscribed.isCompleted) _subscribed.complete();
    return () => _listeners.remove(onEvent);
  }

  void emit(NostrEvent event) {
    for (final listener in List.of(_listeners)) {
      listener(event);
    }
  }
}

class _LiveSubscription {
  final NostrFilter filter;
  final void Function(NostrEvent) onEvent;

  const _LiveSubscription(this.filter, this.onEvent);
}

bool _matches(NostrFilter filter, NostrEvent event) {
  if (!filter.kinds.contains(event.kind)) return false;
  return filter.tags.entries.every((entry) {
    final tagName = entry.key.substring(1);
    return event.tags.any(
      (tag) =>
          tag.isNotEmpty &&
          tag.first == tagName &&
          tag.skip(1).any(entry.value.contains),
    );
  });
}
