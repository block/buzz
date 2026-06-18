import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  test('delivers the same live event to each matching subscription', () async {
    final session = RelaySessionNotifier();
    final firstEvents = <NostrEvent>[];
    final secondEvents = <NostrEvent>[];
    const filter = NostrFilter(
      kinds: EventKind.channelEventKinds,
      tags: {
        '#h': [_channelId],
      },
      limit: 50,
    );

    final firstSubscribe = session.subscribe(filter, firstEvents.add);
    session.debugHandleMessage(['EOSE', 'l-1']);
    final unsubscribeFirst = await firstSubscribe;

    final secondSubscribe = session.subscribe(filter, secondEvents.add);
    session.debugHandleMessage(['EOSE', 'l-2']);
    final unsubscribeSecond = await secondSubscribe;

    final event = _event();
    session.debugHandleMessage(['EVENT', 'l-1', event.toJson()]);
    session.debugHandleMessage(['EVENT', 'l-2', event.toJson()]);
    session.debugFlushEventBuffer();

    expect(firstEvents.map((event) => event.id), [event.id]);
    expect(secondEvents.map((event) => event.id), [event.id]);

    session.debugHandleMessage(['EVENT', 'l-1', event.toJson()]);
    session.debugFlushEventBuffer();

    expect(firstEvents.map((event) => event.id), [event.id]);
    expect(secondEvents.map((event) => event.id), [event.id]);

    unsubscribeFirst();
    unsubscribeSecond();
  });

  test('live subscribe fails when relay closes before ready', () async {
    final session = RelaySessionNotifier();
    const filter = NostrFilter(kinds: [EventKind.agentObserverFrame], limit: 0);

    final subscribe = session.subscribe(filter, (_) {});
    session.debugHandleMessage([
      'CLOSED',
      'l-1',
      'restricted: p-gated events require #p matching your pubkey',
    ]);

    await expectLater(
      subscribe,
      throwsA(
        isA<Exception>().having(
          (error) => error.toString(),
          'message',
          contains('p-gated events require #p'),
        ),
      ),
    );
  });

  test(
    'live onClosed callback runs when relay closes an open subscription',
    () async {
      final session = RelaySessionNotifier();
      final closedMessages = <String>[];
      const filter = NostrFilter(
        kinds: [EventKind.agentObserverFrame],
        limit: 0,
      );

      final subscribe = session.subscribe(
        filter,
        (_) {},
        onClosed: closedMessages.add,
      );
      session.debugHandleMessage(['EOSE', 'l-1']);
      final unsubscribe = await subscribe;
      session.debugHandleMessage([
        'CLOSED',
        'l-1',
        'restricted: no longer valid',
      ]);

      expect(closedMessages, ['restricted: no longer valid']);
      unsubscribe();
    },
  );
}

const _channelId = '11111111-1111-4111-8111-111111111111';

NostrEvent _event() {
  return const NostrEvent(
    id: 'event-1',
    pubkey: 'alice',
    createdAt: 20,
    kind: EventKind.streamMessageV2,
    tags: [
      ['h', _channelId],
    ],
    content: 'hello',
    sig: 'sig',
  );
}
