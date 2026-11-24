import 'dart:async';
import 'dart:io';

import 'package:buzz/shared/push/android_push_wake.dart';
import 'package:buzz/shared/push/push_wake_resolver.dart';
import 'package:buzz/shared/push/push_wake_runner.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/testing.dart';

import '../../helpers/push_wake_fixture.dart';

void main() {
  late Directory directory;
  late PushWakeFixture fixture;

  setUp(() {
    directory = Directory.systemTemp.createTempSync('android-push-wake-');
    fixture = PushWakeFixture(directory);
  });
  tearDown(() => directory.deleteSync(recursive: true));

  group('the wake the native handler receives', () {
    test('carries each notification with where its tap leads', () async {
      final relay = fixture.addCommunity();
      final room = relay.channel();
      final friend = relay.person('Friend');
      final dm = relay.dmChannel();
      final mention = relay.message(room, friend, 'look', mention: relay.me);
      final direct = relay.message(dm, friend, 'hello');
      final handler = PushWakeChannelHandler(
        PushWakeRunner((_) => fixture.wake()),
      );

      final payload = await handler.wake();

      expect(payload['incomplete'], isFalse);
      expect(payload['notifications'], [
        {
          'eventId': direct.id,
          'createdAtMillis': direct.createdAt * 1000,
          'title': 'Friend',
          'body': 'hello',
          'conversationId': '${relay.communityId}:${dm.id}',
          'category': 'directMessage',
          'target': {
            'communityId': relay.communityId,
            'channelId': dm.id,
            'eventId': direct.id,
          },
        },
        {
          'eventId': mention.id,
          'createdAtMillis': mention.createdAt * 1000,
          'title': 'Friend',
          'body': 'look',
          'conversationId': '${relay.communityId}:${room.id}',
          'conversationTitle': '#${room.name}',
          'category': 'mention',
          'target': {
            'communityId': relay.communityId,
            'channelId': room.id,
            'eventId': mention.id,
          },
        },
      ]);
    });

    test('reports a wake that could not read every community', () async {
      final relay = fixture.addCommunity();
      relay.unreachable = true;
      final handler = PushWakeChannelHandler(
        PushWakeRunner((_) => fixture.wake()),
      );

      final payload = await handler.wake();

      expect(payload['notifications'], isEmpty);
      expect(payload['incomplete'], isTrue);
    });

    test(
      'gives a wake that arrives mid-run no notifications of its own',
      () async {
        final relay = fixture.addCommunity();
        final dm = relay.dmChannel();
        final friend = relay.person('Friend');
        relay.message(dm, friend, 'hello');
        final hold = Completer<void>();
        relay.hold = hold;
        final handler = PushWakeChannelHandler(
          PushWakeRunner((_) => fixture.wake()),
        );

        final first = handler.wake();
        final second = handler.wake();
        hold.complete();
        final payloads = await Future.wait([first, second]);

        expect((payloads[0]['notifications'] as List), hasLength(1));
        expect(payloads[1], joinedWake);
      },
    );

    test('after a run ends, the next wake gets its own result', () async {
      final relay = fixture.addCommunity();
      final dm = relay.dmChannel();
      final friend = relay.person('Friend');
      final handler = PushWakeChannelHandler(
        PushWakeRunner((_) => fixture.wake()),
      );
      relay.message(dm, friend, 'one');
      expect((await handler.wake())['notifications'] as List, hasLength(1));

      relay.message(dm, friend, 'two', age: const Duration(minutes: 4));
      final next = await handler.wake();

      expect(
        [for (final n in next['notifications'] as List) (n as Map)['body']],
        ['two'],
      );
    });

    test('a run looks up no longer than the handler has left', () async {
      fixture.addCommunity();
      final budgets = <Duration>[];
      final handler = PushWakeChannelHandler(
        PushWakeRunner((budget) {
          budgets.add(budget);
          return fixture.wake(timeouts: PushWakeTimeouts(total: budget));
        }),
      );
      // A slow engine start leaves less than the runner's own budget; a sent
      // time past it, a missing one, or one already spent fall back to the
      // budget, or to nothing.
      for (final (arguments, expected) in [
        ({'remainingMillis': 1200}, const Duration(milliseconds: 1200)),
        ({'remainingMillis': 60000}, const Duration(seconds: 5)),
        (null, const Duration(seconds: 5)),
        ({'remainingMillis': -5}, Duration.zero),
      ]) {
        budgets.clear();
        await handler.wake(remaining: wakeRemaining(arguments));
        // The pass gets what is left of the run's budget, a hair under it.
        expect(
          (expected - budgets.single).inMilliseconds,
          inInclusiveRange(0, 100),
          reason: '$arguments',
        );
      }
    });
  });

  test('a wake sends the relay the headers the community needs', () async {
    final relay = fixture.addCommunity();
    final dm = relay.dmChannel();
    relay.message(dm, relay.person('Friend'), 'hello');
    final seen = <String?>[];
    final client = MockClient((request) async {
      seen.add(request.headers['Nostr-Federated-Identity']);
      return relay.handle(request);
    });
    final asked = <String>[];

    final result = await resolvePushWake(
      store: fixture.openStore(),
      loadCommunities: () async => [relay.community],
      httpClient: client,
      relayHeaders: (community) async {
        asked.add(community.id);
        return {'Nostr-Federated-Identity': 'Bearer assertion'};
      },
      now: () => pushWakeClock,
    );

    expect(result.notifications, hasLength(1));
    // Once for the wake, whichever queries follow.
    expect(asked, [relay.communityId]);
    expect(seen, isNotEmpty);
    expect(seen.toSet(), {'Bearer assertion'});
  });

  test(
    'a community whose headers cannot be read is reported incomplete',
    () async {
      final refused = fixture.addCommunity();
      refused.message(refused.dmChannel(), refused.person('Friend'), 'hello');
      final reachable = fixture.addCommunity();
      final other = reachable.dmChannel();
      reachable.message(other, reachable.person('Other'), 'hi');

      final result = await resolvePushWake(
        store: fixture.openStore(),
        loadCommunities: () async => [refused.community, reachable.community],
        httpClient: fixture.client,
        relayHeaders: (community) async => community.id == refused.communityId
            ? throw StateError('no session')
            : const {},
        now: () => pushWakeClock,
      );

      expect(result.failedCommunityIds, [refused.communityId]);
      expect([for (final n in result.notifications) n.body], ['hi']);
    },
  );
}
