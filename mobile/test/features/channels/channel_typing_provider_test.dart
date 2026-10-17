import 'package:buzz/features/channels/channel_typing_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../../helpers/thread_ids.dart';

class _Relay extends RelaySessionNotifier {
  late void Function(NostrEvent) receive;
  int calls = 0;
  bool reject = false;
  void Function(String)? close;
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
    calls++;
    close = onClosed;
    if (reject) throw StateError('rejected');
    receive = onEvent;
    active++;
    if (active > peak) peak = active;
    return () {
      active--;
    };
  }
}

void main() {
  test(
    'initial rejection retries with a bound and no uncaught error',
    () async {
      final relay = _Relay()..reject = true;
      final container = ProviderContainer(
        overrides: [relaySessionProvider.overrideWith(() => relay)],
      );
      container.listen(channelTypingProvider('chat'), (_, _) {});
      await container.pump();
      expect(relay.calls, 1);
      for (final seconds in [1, 2, 4, 8]) {
        await Future<void>.delayed(Duration(milliseconds: seconds * 1000 + 20));
      }
      expect(relay.calls, 4);
      container.dispose();
    },
  );
  test(
    'terminal close clears typing and retries; disposal cancels recovery',
    () async {
      final relay = _Relay();
      final container = ProviderContainer(
        overrides: [relaySessionProvider.overrideWith(() => relay)],
      );
      final provider = channelTypingProvider('chat');
      container.listen(provider, (_, _) {});
      await container.pump();
      relay.receive(
        NostrEvent(
          id: 'typing',
          pubkey: 'agent',
          kind: 20002,
          createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
          content: '',
          sig: '',
          tags: [
            ['h', 'chat'],
          ],
        ),
      );
      expect(container.read(provider), hasLength(1));
      relay.close!('terminal');
      await container.pump();
      expect(container.read(provider), isEmpty);
      expect(relay.active, 0);
      await Future<void>.delayed(const Duration(milliseconds: 1100));
      expect(relay.active, 1);
      expect(relay.calls, 2);
      relay.close!('terminal again');
      await container.pump();
      container.dispose();
      await Future<void>.delayed(const Duration(milliseconds: 2100));
      expect(relay.calls, 2);
      expect(relay.active, 0);
    },
  );
  test(
    'terminal closure loops stay bounded until sustained recovery',
    () async {
      final relay = _Relay();
      final container = ProviderContainer(
        overrides: [relaySessionProvider.overrideWith(() => relay)],
      );
      addTearDown(container.dispose);
      container.listen(channelTypingProvider('chat'), (_, _) {});
      await container.pump();
      for (final delay in [1, 2, 4]) {
        final calls = relay.calls;
        relay.close!('rejected after admission');
        await container.pump();
        if (delay > 1) {
          await Future<void>.delayed(const Duration(milliseconds: 1100));
          expect(relay.calls, calls);
        }
        await Future<void>.delayed(
          Duration(milliseconds: delay * 1000 + 50 - (delay > 1 ? 1100 : 0)),
        );
        expect(relay.calls, calls + 1);
      }
      relay.close!('fourth rejection');
      await container.pump();
      await Future<void>.delayed(const Duration(milliseconds: 1100));
      expect(relay.calls, 4);
      relay.setStatus(SessionStatus.disconnected);
      await container.pump();
      relay.setStatus(SessionStatus.connected);
      await container.pump();
      relay.close!('first rejection after reconnect');
      await container.pump();
      await Future<void>.delayed(const Duration(milliseconds: 1100));
      final recoveredCalls = relay.calls;
      await Future<void>.delayed(const Duration(milliseconds: 8100));
      relay.close!('closure after sustained health');
      await container.pump();
      await Future<void>.delayed(const Duration(milliseconds: 1100));
      expect(relay.calls, recoveredCalls + 1);
      expect(relay.active, 1);
    },
  );
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
