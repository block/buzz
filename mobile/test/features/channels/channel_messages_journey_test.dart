import 'package:buzz/features/channels/channel_messages_provider.dart';
import 'package:buzz/features/channels/pending_local_messages_provider.dart';
import 'package:buzz/features/channels/send_message_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/fake_relay.dart';

const _channel = '11111111-1111-4111-8111-111111111111';

void main() {
  test(
    'a channel loads, follows live messages, settles sends and recovers',
    () {
      fakeAsync((async) {
        final relay = FakeRelay();
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        NostrEvent message(String text, int createdAt) => relay.sign(
          kind: EventKind.streamMessage,
          content: text,
          createdAt: createdAt,
          tags: [
            ['h', _channel],
          ],
        );
        relay.seed([message('first', now - 20), message('second', now - 10)]);

        final container = ProviderContainer(overrides: relay.overrides);
        addTearDown(container.dispose);
        final messages = container.listen(
          channelMessagesProvider(_channel),
          (_, _) {},
        );
        List<String> timeline() => [
          for (final event in messages.read().requireValue) event.content,
        ];
        Set<String> pending() =>
            container.read(pendingLocalMessagesProvider(_channel)).keys.toSet();
        void settle() => async.elapse(const Duration(milliseconds: 50));
        // The live subscription starts at the second it opens, so live
        // messages are dated from the filter it actually installed.
        int liveSince() =>
            relay.reqs
                    .expand((req) => req.skip(2).cast<Map<String, dynamic>>())
                    .lastWhere(
                      (filter) =>
                          (filter['kinds'] as List).contains(
                            EventKind.streamMessage,
                          ) &&
                          ((filter['#h'] as List?)?.contains(_channel) ??
                              false) &&
                          filter['since'] != null,
                    )['since']
                as int;

        settle();
        expect(
          container.read(relaySessionProvider).status,
          SessionStatus.connected,
        );
        expect(timeline(), ['first', 'second']);
        expect(relay.queries, hasLength(1), reason: 'read the channel window');
        expect(
          relay.reqs.where((req) => (req[1] as String).startsWith('h-')),
          isEmpty,
          reason: 'no fallback to websocket history',
        );

        final since = liveSince();
        relay.deliver(message('from a teammate', since));
        relay.deliver(message('from before the subscription', since - 1));
        settle();
        expect(timeline(), contains('from a teammate'));
        expect(timeline(), isNot(contains('from before the subscription')));

        // Pending until OK, settled by OK alone, one row after the echo.
        final ok = relay.holdOk();
        final echo = relay.holdEcho();
        var sent = false;
        container
            .read(sendMessageProvider)(channelId: _channel, content: 'hello')
            .then((_) => sent = true);
        settle();
        final sentEvent = relay.published.single;
        expect(sentEvent.kind, EventKind.streamMessage);
        expect(sentEvent.pubkey, relay.pubkey);
        expect(sentEvent.getTagValue('h'), _channel);
        expect(pending(), {sentEvent.id});
        expect(timeline(), contains('hello'));

        ok.complete();
        settle();
        expect(sent, isTrue);
        expect(pending(), isEmpty);
        expect(timeline().where((text) => text == 'hello'), hasLength(1));

        echo.complete();
        settle();
        expect(timeline().where((text) => text == 'hello'), hasLength(1));

        // A rejected send leaves no row and is not stored by the relay.
        relay.rejectNext('blocked: not allowed');
        Object? failure;
        container
            .read(sendMessageProvider)(channelId: _channel, content: 'nope')
            .catchError((Object error) => failure = error);
        settle();
        expect(failure, isNotNull);
        expect(pending(), isEmpty);
        expect(timeline(), isNot(contains('nope')));
        expect(relay.stored.map((e) => e.content), isNot(contains('nope')));

        // After a network drop the session reconnects, recovers what it missed
        // and follows the channel again, without duplicating known rows.
        relay.drop();
        relay.seed([message('while offline', since)]);
        async.elapse(const Duration(seconds: 2));
        expect(relay.sockets, hasLength(2));
        expect(timeline(), contains('while offline'));
        relay.deliver(message('after reconnect', liveSince()));
        settle();
        expect(timeline(), contains('after reconnect'));
        expect(timeline().where((text) => text == 'hello'), hasLength(1));
      });
    },
  );
}
