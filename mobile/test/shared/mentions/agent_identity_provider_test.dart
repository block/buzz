import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
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
  final List<void Function(NostrEvent)> _listeners = [];
  final Completer<void> _subscribed = Completer<void>();
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
