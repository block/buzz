import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/agent_activity/working_bots_provider.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
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
