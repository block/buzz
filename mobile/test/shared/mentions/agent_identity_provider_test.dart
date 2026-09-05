import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/features/channels/agent_activity/working_bots_provider.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  group('NIP-IA archived identity snapshots', () {
    test(
      'accepts a valid relay-signed snapshot and ignores malformed p tags',
      () {
        final event = _archiveSnapshot(
          secretKey: _relay.secret,
          createdAt: 100,
          pubkeys: [_archivedA.toUpperCase(), 'not-a-pubkey'],
        );

        final parsed = parseArchivedIdentitySnapshot(event, _relayPubkey);

        expect(parsed, isNotNull);
        expect(parsed!.pubkeys, {_archivedA});
      },
    );

    test(
      'rejects wrong author, invalid signature, and missing NIP-70 marker',
      () {
        final wrongAuthor = _archiveSnapshot(
          secretKey: _otherRelay.secret,
          createdAt: 100,
          pubkeys: [_archivedA],
        );
        final valid = _archiveSnapshot(
          secretKey: _relay.secret,
          createdAt: 101,
          pubkeys: [_archivedA],
        );
        final invalidSignature = NostrEvent(
          id: valid.id,
          pubkey: valid.pubkey,
          createdAt: valid.createdAt,
          kind: valid.kind,
          tags: valid.tags,
          content: valid.content,
          sig: '0' * 128,
        );
        final missingMarker = _archiveSnapshot(
          secretKey: _relay.secret,
          createdAt: 102,
          pubkeys: [_archivedA],
          includeMarker: false,
        );

        expect(
          parseArchivedIdentitySnapshot(wrongAuthor, _relayPubkey),
          isNull,
        );
        expect(
          parseArchivedIdentitySnapshot(invalidSignature, _relayPubkey),
          isNull,
        );
        expect(
          parseArchivedIdentitySnapshot(missingMarker, _relayPubkey),
          isNull,
        );
      },
    );

    test('newer snapshot wins and same-time lower event id breaks ties', () {
      final old = _archiveSnapshot(
        secretKey: _relay.secret,
        createdAt: 100,
        pubkeys: [_archivedA],
      );
      final newer = _archiveSnapshot(
        secretKey: _relay.secret,
        createdAt: 101,
        pubkeys: [_archivedB],
      );
      expect(
        latestArchivedIdentitySnapshot([newer, old], _relayPubkey)!.pubkeys,
        {_archivedB},
      );

      final sameTimeA = _archiveSnapshot(
        secretKey: _relay.secret,
        createdAt: 102,
        pubkeys: [_archivedA],
      );
      final sameTimeB = _archiveSnapshot(
        secretKey: _relay.secret,
        createdAt: 102,
        pubkeys: [_archivedB],
      );
      final expected = sameTimeA.id.compareTo(sameTimeB.id) < 0
          ? _archivedA
          : _archivedB;
      expect(
        latestArchivedIdentitySnapshot([
          sameTimeA,
          sameTimeB,
        ], _relayPubkey)!.pubkeys,
        {expected},
      );
    });

    test(
      'reads relay identity from NIP-11 and rejects malformed metadata',
      () async {
        final validClient = MockClient(
          (_) async => http.Response(jsonEncode({'self': _relayPubkey}), 200),
        );
        final malformedClient = MockClient(
          (_) async => http.Response(jsonEncode({'self': 'invalid'}), 200),
        );

        expect(
          await fetchRelayIdentityPubkey(validClient, 'https://relay.example'),
          _relayPubkey,
        );
        expect(
          await fetchRelayIdentityPubkey(
            malformedClient,
            'https://relay.example',
          ),
          isNull,
        );
      },
    );

    test('live valid replacement refreshes the provider output', () async {
      final initial = _archiveSnapshot(
        secretKey: _relay.secret,
        createdAt: 100,
        pubkeys: [_archivedA],
      );
      final replacement = _archiveSnapshot(
        secretKey: _relay.secret,
        createdAt: 101,
        pubkeys: [_archivedB],
      );
      final relaySession = _ArchiveRelaySessionNotifier(initial);
      final client = MockClient(
        (_) async => http.Response(jsonEncode({'self': _relayPubkey}), 200),
      );
      final container = ProviderContainer(
        overrides: [
          relayConfigProvider.overrideWith(_TestRelayConfigNotifier.new),
          relaySessionProvider.overrideWith(() => relaySession),
          archivedIdentityHttpClientProvider.overrideWithValue(client),
        ],
      );
      addTearDown(container.dispose);
      final keepAlive = container.listen(
        archivedIdentityPubkeysProvider,
        (_, _) {},
        fireImmediately: true,
      );
      addTearDown(keepAlive.close);

      expect(await container.read(archivedIdentityPubkeysProvider.future), {
        _archivedA,
      });
      await relaySession.subscribed;
      relaySession.emit(replacement);
      await _pumpEventQueue();

      expect(container.read(archivedIdentityPubkeysProvider).value, {
        _archivedB,
      });
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
    expect(relaySession.liveFilters.single.tags['#d'], [_channelId]);
    expect(relaySession.liveFilters.single.tags['#h'], isNull);

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

  test('surfaces bot-role subscription setup failure', () async {
    final relaySession = _MembershipRelaySessionNotifier([
      _membershipEvent(role: 'member'),
    ], subscribeError: StateError('subscription unavailable'));
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => relaySession)],
    );
    addTearDown(container.dispose);
    final keepAlive = container.listen(
      channelMembershipUpdateProvider(_channelId),
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(keepAlive.close);

    await _pumpEventQueue();

    final state = container.read(channelMembershipUpdateProvider(_channelId));
    expect(state.isReady, isFalse);
    expect(state.error, isA<StateError>());
  });

  test('surfaces terminal bot-role subscription closure', () async {
    final relaySession = _MembershipRelaySessionNotifier([
      _membershipEvent(role: 'member'),
    ]);
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => relaySession)],
    );
    addTearDown(container.dispose);
    final keepAlive = container.listen(
      channelMembershipUpdateProvider(_channelId),
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(keepAlive.close);

    await relaySession.subscribed;
    await _pumpEventQueue();
    expect(
      container.read(channelMembershipUpdateProvider(_channelId)).isReady,
      isTrue,
    );

    relaySession.closeSubscription('unsupported filter');
    await _pumpEventQueue();

    final state = container.read(channelMembershipUpdateProvider(_channelId));
    expect(state.isReady, isFalse);
    expect(state.error, isA<Exception>());
  });

  test('fails closed while bot-role subscription retries', () async {
    final relaySession = _MembershipRelaySessionNotifier([
      _membershipEvent(role: 'member'),
    ]);
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => relaySession)],
    );
    addTearDown(container.dispose);
    final keepAlive = container.listen(
      channelMembershipUpdateProvider(_channelId),
      (_, _) {},
      fireImmediately: true,
    );
    addTearDown(keepAlive.close);

    await relaySession.subscribed;
    await _pumpEventQueue();
    expect(
      container.read(channelMembershipUpdateProvider(_channelId)).isReady,
      isTrue,
    );

    relaySession.setSubscriptionStatus(RelaySubscriptionStatus.retrying);
    await _pumpEventQueue();
    expect(
      container.read(channelMembershipUpdateProvider(_channelId)).isReady,
      isFalse,
    );

    relaySession.setSubscriptionStatus(RelaySubscriptionStatus.ready);
    await _pumpEventQueue();
    final recovered = container.read(
      channelMembershipUpdateProvider(_channelId),
    );
    expect(recovered.isReady, isTrue);
    expect(recovered.error, isNull);
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
final _relay = nostr.Keys.generate();
final _otherRelay = nostr.Keys.generate();
final _relayPubkey = _relay.public;
const _archivedA =
    '1111111111111111111111111111111111111111111111111111111111111111';
const _archivedB =
    '2222222222222222222222222222222222222222222222222222222222222222';

NostrEvent _archiveSnapshot({
  required String secretKey,
  required int createdAt,
  required List<String> pubkeys,
  bool includeMarker = true,
}) {
  final event = nostr.Event.from(
    kind: EventKind.archivedIdentities,
    content: '',
    tags: [
      if (includeMarker) ['-'],
      for (final pubkey in pubkeys) ['p', pubkey],
    ],
    secretKey: secretKey,
    createdAt: createdAt,
    verify: false,
  );
  return NostrEvent.fromJson(event.toMap());
}

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
  final Object? subscribeError;
  final List<NostrFilter> liveFilters = [];
  final List<_LiveSubscription> _subscriptions = [];
  final Completer<void> _subscribed = Completer<void>();
  var unsubscribeCount = 0;
  var _membershipIndex = 0;

  _MembershipRelaySessionNotifier(this._memberships, {this.subscribeError});

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
  Future<void Function()> subscribeWithStatus(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
    required void Function(RelaySubscriptionStatus status) onStatusChanged,
  }) async {
    if (subscribeError case final error?) throw error;
    liveFilters.add(filter);
    final subscription = _LiveSubscription(
      filter,
      onEvent,
      onClosed,
      onStatusChanged,
    );
    _subscriptions.add(subscription);
    onStatusChanged(RelaySubscriptionStatus.ready);
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

  void closeSubscription(String message) {
    for (final subscription in List.of(_subscriptions)) {
      subscription.onClosed?.call(message);
    }
  }

  void setSubscriptionStatus(RelaySubscriptionStatus status) {
    for (final subscription in List.of(_subscriptions)) {
      subscription.onStatusChanged?.call(status);
    }
  }
}

class _TestRelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(baseUrl: 'https://relay.example');
}

class _ArchiveRelaySessionNotifier extends RelaySessionNotifier {
  final NostrEvent initial;
  final Completer<void> _subscribed = Completer<void>();
  void Function(NostrEvent)? _onEvent;

  _ArchiveRelaySessionNotifier(this.initial);

  Future<void> get subscribed => _subscribed.future;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [initial];

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    _onEvent = onEvent;
    if (!_subscribed.isCompleted) _subscribed.complete();
    return () => _onEvent = null;
  }

  void emit(NostrEvent event) => _onEvent?.call(event);
}

class _LiveSubscription {
  final NostrFilter filter;
  final void Function(NostrEvent) onEvent;
  final void Function(String message)? onClosed;
  final void Function(RelaySubscriptionStatus status)? onStatusChanged;

  const _LiveSubscription(
    this.filter,
    this.onEvent,
    this.onClosed,
    this.onStatusChanged,
  );
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
