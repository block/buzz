import 'dart:async';
import 'package:flutter/material.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_actions_sheet.dart';
import 'package:buzz/features/channels/mentions/mention_candidates_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/widgets/buzz_action_tile.dart';
import '../../helpers/widget_helpers.dart';

import 'package:nostr/nostr.dart' as nostr;
import '../crypto/nip_oa_test.dart' show authTag, profile;

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/agent_activity/working_bots_provider.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  final owner = nostr.Keys.generate();
  final agent = nostr.Keys.generate();
  final owned = profile(agent, [authTag(owner, agent.public)]);
  final revoked = profile(agent, [], createdAt: 101);
  // Canonical positive/revocation/tie projection is exercised through both
  // owner and search providers in channel_management_provider_test.dart.
  for (final details in [false, true]) {
    testWidgets(
      '${details ? 'Details' : 'Actions'} follows live owner authority',
      (tester) async {
        await tester.binding.setSurfaceSize(const Size(900, 2400));
        addTearDown(() => tester.binding.setSurfaceSize(null));
        final restored = profile(agent, [
          authTag(owner, agent.public),
        ], createdAt: 103);
        final invalid = profile(agent, [
          authTag(owner, owner.public),
        ], createdAt: 104);
        final session = _MembershipRelaySessionNotifier(
          [owned, profile(agent, [], kind: 10100)],
          profiles: true,
          autoReady: false,
        );
        final channel = Channel(
          id: _channelId,
          name: 'proof',
          channelType: 'stream',
          visibility: 'open',
          description: '',
          createdBy: agent.public,
          createdAt: DateTime(2025),
          memberCount: 1,
          isMember: true,
        );
        await tester.pumpWidget(
          WidgetHelpers.testable(
            overrides: [
              relaySessionProvider.overrideWith(() => session),
              relayConfigProvider.overrideWith(_FixedRelayConfig.new),
              currentPubkeyProvider.overrideWithValue(owner.public),
              channelMembersProvider(_channelId).overrideWith(
                (ref) async => [
                  ChannelMember(
                    pubkey: agent.public,
                    role: 'owner',
                    joinedAt: DateTime(2025),
                  ),
                ],
              ),
            ],
            child: details
                ? ChannelDetailsPage(
                    channel: channel,
                    currentPubkey: owner.public,
                    onMemberTap: (_, _) {},
                  )
                : ChannelActionsSheet(channel: channel, isUnread: false),
          ),
        );
        await tester.pumpAndSettle();
        final container = ProviderScope.containerOf(
          tester.element(
            find.byType(details ? ChannelDetailsPage : ChannelActionsSheet),
          ),
        );
        final candidates = mentionCandidatesProvider((
          channelId: _channelId,
          query: '',
        ));
        final listener = container.listen(candidates, (_, _) {});
        addTearDown(listener.close);
        await tester.pumpAndSettle();
        Future<void> controls(bool allowed) async {
          await tester.pumpAndSettle();
          if (details) {
            final edit = find.byKey(
              const ValueKey('channel-details-edit-action'),
            );
            expect(tester.widget<BuzzActionTile>(edit).isEnabled, allowed);
          } else {
            expect(
              find.text('Archive channel'),
              allowed ? findsOneWidget : findsNothing,
            );
            expect(
              find.text('Delete channel'),
              allowed ? findsOneWidget : findsNothing,
            );
          }
          expect(
            container.read(candidates).single.ownerPubkey,
            allowed ? owner.public : isNull,
          );
        }

        // Acquisition alone is not EOSE, even with a positive cached profile.
        await controls(false);
        session.autoReady = true;
        session.setSubscriptionStatus(RelaySubscriptionStatus.ready);
        await controls(true);
        session.closeProfiles();
        await controls(false);
        expect(container.read(userCacheProvider.notifier).profileOwners, {
          agent.public: owner.public,
        });
        session._memberships[0] = revoked;
        session.connection(SessionStatus.disconnected);
        await tester.pumpAndSettle();
        session.connection(SessionStatus.connected);
        await controls(false);
        for (final event in [revoked, restored, invalid, owned]) {
          session.emit(event);
          await controls(event.id == restored.id);
          if (event.id == restored.id) {
            session.setSubscriptionStatus(RelaySubscriptionStatus.retrying);
            await controls(false);
            session.setSubscriptionStatus(RelaySubscriptionStatus.ready);
            await controls(true);
            session.connection(SessionStatus.disconnected);
            await controls(false);
            session.connection(SessionStatus.connected);
            await controls(true);
          }
        }
        final cache = container.read(userCacheProvider.notifier);
        await cache.refresh([agent.public]);
        // A new connection rebuilds the profile producer against stale history.
        session.connection(SessionStatus.disconnected);
        await tester.pumpAndSettle();
        session.connection(SessionStatus.connected);
        await tester.pumpAndSettle();
        session.emit(owned);
        await controls(false);
        expect(cache.profileOwners, isEmpty);
        await tester.pumpWidget(const SizedBox());
      },
    );
  }
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
  final bool profiles;
  bool autoReady;
  final List<NostrFilter> liveFilters = [];
  final List<_LiveSubscription> _subscriptions = [];
  final Completer<void> _subscribed = Completer<void>();
  var unsubscribeCount = 0;
  var _membershipIndex = 0;

  _MembershipRelaySessionNotifier(
    this._memberships, {
    this.subscribeError,
    this.profiles = false,
    this.autoReady = true,
  });

  void connection(SessionStatus status) => state = SessionState(status: status);

  Future<void> get subscribed => _subscribed.future;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    return profiles
        ? _memberships
              .where((event) => filter.kinds.contains(event.kind))
              .toList()
        : [_memberships[_membershipIndex++]];
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
    if (profiles) {
      for (final event in _memberships.where(
        (event) => _matches(filter, event),
      )) {
        onEvent(event);
      }
    }
    if (autoReady) onStatusChanged(RelaySubscriptionStatus.ready);
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

  void closeProfiles() {
    for (final sub in List.of(_subscriptions)) {
      if (!sub.filter.kinds.contains(0)) continue;
      sub.onClosed?.call('restricted: no longer valid');
      _subscriptions.remove(sub);
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

class _FixedRelayConfig extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(baseUrl: 'https://relay.invalid');
}
