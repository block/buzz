import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_identity_names_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _channelId = '11111111-1111-4111-8111-111111111111';
final _human = 'c' * 64;
final _bot = 'd' * 64;

void main() {
  late _MembershipRelaySession session;
  late ProviderContainer container;

  setUp(() {
    session = _MembershipRelaySession();
    container = ProviderContainer(
      retry: (_, _) => null,
      overrides: [
        relaySessionProvider.overrideWith(() => session),
        relayConfigProvider.overrideWith(_TestRelayConfigNotifier.new),
        myPubkeyProvider.overrideWith((ref) => ref.watch(_accountProvider)),
        channelsProvider.overrideWith(_EmptyChannelsNotifier.new),
        userCacheProvider.overrideWith(
          () => _FixedUserCacheNotifier({
            _human: UserProfile(pubkey: _human, displayName: 'Honey'),
            _bot: UserProfile(pubkey: _bot, displayName: 'Honey'),
          }),
        ),
      ],
    );
    addTearDown(container.dispose);
  });

  ProviderSubscription<Object?> listenNames() {
    final subscription = container.listen(
      channelIdentityNamesProvider(_channelId),
      (_, _) {},
    );
    addTearDown(subscription.close);
    return subscription;
  }

  List<String> labels() {
    final names = container.read(channelIdentityNamesProvider(_channelId));
    return [names.labelFor(_human), names.labelFor(_bot)];
  }

  Future<void> settle() async {
    for (var i = 0; i < 5; i++) {
      await container.pump();
      await Future<void>.delayed(Duration.zero);
    }
  }

  test('keeps the same-scope roster while membership reloads', () async {
    listenNames();
    await settle();
    expect(labels(), ['Honey', 'Honey (agent)']);
    expect(session.membershipEvents, isNotNull);

    // A live kind:39002 update reloads the roster. Hold that reload open.
    final reload = session.holdMemberFetches();
    session.membershipEvents!(_membersEvent());
    await settle();
    expect(session.heldMemberFetches, isPositive);
    expect(labels(), ['Honey', 'Honey (agent)']);

    reload.complete();
    await settle();
    expect(labels(), ['Honey', 'Honey (agent)']);
  });

  test('uses cached bot roles when the relay is offline', () async {
    final subscription = listenNames();
    await settle();
    expect(labels(), ['Honey', 'Honey (agent)']);

    // Unmount, go offline, and mount again from the cached roster. The
    // relay-backed bot lookup and agent directory are empty while offline.
    subscription.close();
    await settle();
    session.setStatus(SessionStatus.disconnected);
    await settle();
    listenNames();
    await settle();

    final names = container.read(channelIdentityNamesProvider(_channelId));
    expect(names.candidates, {_human, _bot});
    expect(labels(), ['Honey', 'Honey (agent)']);
  });

  for (final reset in _scopeResets.entries) {
    test('drops the roster when the ${reset.key} changes', () async {
      listenNames();
      await settle();
      expect(
        container.read(channelIdentityNamesProvider(_channelId)).candidates,
        {_human, _bot},
      );

      final reload = session.holdMemberFetches();
      reset.value(container);
      await settle();
      expect(session.heldMemberFetches, isPositive);
      expect(
        container.read(channelIdentityNamesProvider(_channelId)).candidates,
        isEmpty,
      );
      reload.complete();
      await settle();
    });
  }
}

final _scopeResets = <String, void Function(ProviderContainer)>{
  'relay': (container) => container
      .read(relayConfigProvider.notifier)
      .update(baseUrl: 'https://second-community.example'),
  'account': (container) =>
      container.read(_accountProvider.notifier).set('e' * 64),
};

NostrEvent _membersEvent() => NostrEvent(
  id: 'members',
  pubkey: 'owner',
  createdAt: 1,
  kind: 39002,
  tags: [
    const ['d', _channelId],
    ['p', _human, '', 'member'],
    ['p', _bot, '', 'bot'],
  ],
  content: '',
  sig: 'sig',
);

final _accountProvider = NotifierProvider<_AccountNotifier, String?>(
  _AccountNotifier.new,
);

class _AccountNotifier extends Notifier<String?> {
  @override
  String? build() => 'a' * 64;

  void set(String pubkey) => state = pubkey;
}

class _TestRelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() =>
      const RelayConfig(baseUrl: 'https://first-community.example');

  @override
  void update({required String baseUrl, String? nsec}) {
    state = RelayConfig(baseUrl: baseUrl, nsec: nsec);
  }
}

class _EmptyChannelsNotifier extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => const [];
}

class _FixedUserCacheNotifier extends UserCacheNotifier {
  _FixedUserCacheNotifier(this._users);

  final Map<String, UserProfile> _users;

  @override
  Map<String, UserProfile> build() => _users;

  @override
  Future<bool> preload(List<String> pubkeys) async => true;
}

/// Serves the channel's kind:39002 roster, can hold member fetches open, and
/// exposes the live membership subscription's event callback.
class _MembershipRelaySession extends RelaySessionNotifier {
  void Function(NostrEvent)? membershipEvents;
  Completer<void>? _hold;
  int heldMemberFetches = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  void setStatus(SessionStatus status) => state = SessionState(status: status);

  Completer<void> holdMemberFetches() => _hold = Completer<void>();

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (!(filter.kinds.contains(39002))) return const [];
    if (_hold case final hold?) {
      heldMemberFetches++;
      await hold.future;
    }
    return [_membersEvent()];
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async => const [];

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async => () {};

  @override
  Future<void Function()> subscribeWithStatus(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
    required void Function(RelaySubscriptionStatus status) onStatusChanged,
  }) async {
    if (filter.kinds.contains(39002)) membershipEvents = onEvent;
    onStatusChanged(RelaySubscriptionStatus.ready);
    return () {};
  }
}
