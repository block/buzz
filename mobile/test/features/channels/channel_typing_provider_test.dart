import 'package:buzz/features/channels/channel_typing_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../../helpers/thread_ids.dart';

class _Relay extends RelaySessionNotifier {
  late void Function(NostrEvent) receive;
  int active = 0;
  int peak = 0;
  void setStatus(SessionStatus status) => state = SessionState(status: status);
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);
  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String)? onClosed,
  }) async {
    expect(filter.kinds, containsAll([20002, 9, 40002, 40008]));
    receive = onEvent;
    active++;
    if (active > peak) peak = active;
    return () {
      active--;
    };
  }
}

void main() {
  test('transport rebuilds release each typing subscription', () async {
    final relay = _Relay();
    final container = ProviderContainer(
      overrides: [relaySessionProvider.overrideWith(() => relay)],
    );
    container.listen(channelTypingProvider('chat'), (_, _) {});
    await container.pump();
    for (var i = 0; i < 4; i++) {
      relay.setStatus(SessionStatus.disconnected);
      await container.pump();
      expect(relay.active, 0);
      relay.setStatus(SessionStatus.connected);
      await container.pump();
      expect(relay.active, 1);
    }
    expect(relay.peak, 1);
    container.dispose();
    await Future<void>.delayed(Duration.zero);
    expect(relay.active, 0);
  });
  test(
    'reply clears its scope immediately and suppresses late pulses',
    () async {
      final relay = _Relay();
      final container = ProviderContainer(
        overrides: [relaySessionProvider.overrideWith(() => relay)],
      );
      addTearDown(container.dispose);
      final provider = channelTypingProvider('chat');
      container.read(provider);
      await Future<void>.value();
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      void emit(int kind, String agent, String root, int time) => relay.receive(
        NostrEvent(
          id: '$kind-$agent-$root-$time',
          pubkey: agent,
          kind: kind,
          createdAt: time,
          content: '',
          sig: '',
          tags: [
            ['h', 'chat'],
            ['e', root, '', 'root'],
            ['e', threadIdParent, '', 'reply'],
          ],
        ),
      );
      emit(20002, 'a', threadIdRoot, now - 2);
      emit(20002, 'a', threadIdOtherRoot, now - 2);
      emit(20002, 'b', threadIdRoot, now - 2);
      expect(container.read(provider), hasLength(3));
      emit(9, 'a', threadIdRoot, now - 1);
      expect(container.read(provider).map((e) => (e.pubkey, e.threadHeadId)), [
        ('a', threadIdOtherRoot),
        ('b', threadIdRoot),
      ]);
      emit(
        20002,
        'a',
        threadIdRoot,
        now,
      ); // Newer but inside desktop's quiet interval.
      emit(20002, 'a', threadIdRoot, now - 2); // Delayed old pulse.
      expect(container.read(provider), hasLength(2));
      emit(
        20002,
        'c',
        threadIdRoot,
        now - 9,
      ); // Expired history is not live typing.
      expect(container.read(provider), hasLength(2));
    },
  );
}
