import 'dart:async';
import 'dart:io';

import 'package:buzz/shared/push/push_wake_resolver.dart';

import 'package:buzz/shared/push/push_wake_runner.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../helpers/push_wake_fixture.dart';

void main() {
  late Directory directory;
  late PushWakeFixture fixture;

  setUp(() {
    directory = Directory.systemTemp.createTempSync('push-wake-');
    fixture = PushWakeFixture(directory);
  });
  tearDown(() => directory.deleteSync(recursive: true));

  test('a DM and a mention notify, and nothing else does', () async {
    final relay = fixture.addCommunity();
    final dm = relay.dmChannel();
    final muted = relay.dmChannel();
    final room = relay.channel();
    final friend = relay.person('Friend');
    final bystander = relay.person('Bystander');
    relay.mute(muted);

    final direct = relay.message(dm, friend, 'hello **there**');
    final mention = relay.message(
      room,
      friend,
      'look at this',
      mention: relay.me,
    );
    // Not notifying: my own message from another device, a muted DM, a
    // hellthread tagging more people than the limit, chatter that doesn't
    // mention me, and a message with no text.
    relay.message(dm, relay.me, 'sent from my laptop');
    relay.message(muted, friend, 'muted');
    relay.message(
      room,
      friend,
      'everyone!',
      mention: relay.me,
      extraMentions: List.generate(20, (_) => nostr.Keys.generate().public),
    );
    relay.message(room, bystander, 'unrelated');
    relay.message(dm, friend, '`` ``');

    final result = await fixture.wake();

    expect(result.incomplete, isFalse);
    expect(result.notifications.map((n) => n.eventId), [mention.id, direct.id]);
    final [first, second] = result.notifications;
    expect(first.category, PushWakeCategory.mention);
    expect(first.title, 'Friend');
    expect(first.body, 'look at this');
    expect(first.conversationTitle, '#${room.name}');
    expect(first.target.channelId, room.id);
    expect(first.target.eventId, mention.id);
    expect(second.category, PushWakeCategory.directMessage);
    expect(second.body, 'hello there');
    expect(second.conversationTitle, isNull);
    expect(second.conversationId, isNot(first.conversationId));
  });

  test('shown messages stay shown, and a missed wake is recovered', () async {
    final relay = fixture.addCommunity();
    final first = relay.dmChannel();
    final second = relay.dmChannel();
    final friend = relay.person('Friend');
    // History from before this install knew about the community.
    relay.message(first, friend, 'ancient', age: const Duration(hours: 2));
    final arrived = relay.message(first, friend, 'one');

    expect((await fixture.wake()).notifications.map((n) => n.eventId), [
      arrived.id,
    ]);
    expect((await fixture.wake()).notifications, isEmpty);

    // Several messages arrive across two conversations while no wake does.
    final b = relay.message(second, friend, 'two');
    final c = relay.message(first, friend, 'three');
    final d = relay.message(second, friend, 'four');
    final recovered = await fixture.wake();
    expect(recovered.notifications.map((n) => n.eventId), [d.id, c.id, b.id]);
    expect(
      recovered.notifications.map((n) => n.conversationId).toSet(),
      hasLength(2),
    );
    expect((await fixture.wake()).notifications, isEmpty);

    // Later wakes fetch from the last shown message, not from the start.
    final since = relay.queries.last.first['since'] as int;
    expect(
      since,
      greaterThan(
        pushWakeClock
                .subtract(const Duration(hours: 1))
                .millisecondsSinceEpoch ~/
            1000,
      ),
    );

    // A message stamped earlier than one already shown (clock skew or
    // delivery order) still notifies, once.
    final older = relay.message(
      first,
      friend,
      'five',
      age: const Duration(minutes: 10),
    );
    expect(older.createdAt, lessThan(d.createdAt));
    expect((await fixture.wake()).notifications.map((n) => n.eventId), [
      older.id,
    ]);
    expect((await fixture.wake()).notifications, isEmpty);
  });

  test('every community notifies, each from its own relay', () async {
    final relays = [fixture.addCommunity(), fixture.addCommunity()];
    final events = <String>[];
    for (final relay in relays) {
      final dm = relay.dmChannel();
      events.add(relay.message(dm, relay.person('Friend'), 'hi').id);
    }

    final result = await fixture.wake();

    expect(result.notifications.map((n) => n.eventId).toSet(), events.toSet());
    expect(
      result.notifications.map((n) => n.target.communityId).toSet(),
      relays.map((r) => r.communityId).toSet(),
    );
  });

  test(
    'a burst is bounded per conversation and overall, and not repeated',
    () async {
      final relay = fixture.addCommunity();
      final friend = relay.person('Friend');
      final channels = List.generate(4, (_) => relay.dmChannel());
      for (final channel in channels) {
        for (var i = 0; i < 12; i++) {
          relay.message(channel, friend, 'message $i');
        }
      }

      final result = await fixture.wake();

      expect(result.notifications, hasLength(pushWakeMaxNotifications));
      final counts = <String, int>{};
      for (final n in result.notifications) {
        counts.update(n.conversationId, (c) => c + 1, ifAbsent: () => 1);
      }
      expect(
        counts.values.every((c) => c <= pushWakeMaxPerConversation),
        isTrue,
      );
      // The messages left out are recorded as shown, so they don't resurface.
      expect((await fixture.wake()).notifications, isEmpty);
    },
  );

  group('slow and unreachable relays', () {
    const timeouts = PushWakeTimeouts(
      request: Duration(milliseconds: 150),
      metadata: Duration(milliseconds: 100),
      total: Duration(milliseconds: 600),
    );

    test('return what they have inside the budget, and recover', () async {
      final slow = fixture.addCommunity();
      final healthy = fixture.addCommunity();
      final dead = fixture.addCommunity();
      final slowMessage = slow.message(
        slow.dmChannel(),
        slow.person('Friend'),
        'slow',
      );
      final healthyMessage = healthy.message(
        healthy.dmChannel(),
        healthy.person('Friend'),
        'fast',
      );
      slow.silent = true;
      dead.unreachable = true;

      final watch = Stopwatch()..start();
      final result = await fixture.wake(timeouts: timeouts);

      expect(watch.elapsed, lessThan(const Duration(seconds: 2)));
      expect(result.notifications.map((n) => n.eventId), [healthyMessage.id]);
      expect(result.incomplete, isTrue);
      expect(
        result.failedCommunityIds,
        [dead.communityId, slow.communityId]..sort(),
      );

      // Nothing was recorded for the communities that failed, so the next
      // wake finds their messages once they answer.
      slow.silent = false;
      dead.unreachable = false;
      final next = await fixture.wake(timeouts: timeouts);
      expect(next.incomplete, isFalse);
      expect(next.notifications.map((n) => n.eventId), [slowMessage.id]);
    });

    test('with nothing found say so, so the caller can fall back', () async {
      final relay = fixture.addCommunity()..unreachable = true;
      relay.message(relay.dmChannel(), relay.person('Friend'), 'hi');

      final result = await fixture.wake(timeouts: timeouts);

      expect(result.notifications, isEmpty);
      expect(result.incomplete, isTrue);
    });

    test('the whole wake stops at its total budget', () async {
      final relay = fixture.addCommunity()..silent = true;
      relay.message(relay.dmChannel(), relay.person('Friend'), 'hi');
      const patient = PushWakeTimeouts(
        request: Duration(seconds: 30),
        total: Duration(milliseconds: 300),
      );

      final watch = Stopwatch()..start();
      final result = await fixture.wake(timeouts: patient);

      expect(watch.elapsed, lessThan(const Duration(seconds: 2)));
      expect(result.failedCommunityIds, [relay.communityId]);
    });
  });

  group('overlapping writers', () {
    Future<void> untilQueries(FakePushRelay relay, int count) async {
      while (relay.queries.length < count) {
        await Future<void>.delayed(Duration.zero);
      }
    }

    test('two wakes in different isolates never notify twice', () async {
      final relay = fixture.addCommunity();
      final dm = relay.dmChannel();
      final friend = relay.person('Friend');
      final sent = [
        for (var i = 0; i < 5; i++) relay.message(dm, friend, 'message $i').id,
      ];
      final release = relay.hold = Completer<void>();

      // Each opens its own store over the same files.
      final wakes = [
        fixture.wake(store: fixture.openStore()),
        fixture.wake(store: fixture.openStore()),
      ];
      await untilQueries(relay, 2);
      release.complete();
      final results = await Future.wait(wakes);

      final shown = [
        for (final result in results)
          ...result.notifications.map((n) => n.eventId),
      ];
      expect(shown, unorderedEquals(sent));
    });

    test('the app marking a message seen during a run is respected', () async {
      final relay = fixture.addCommunity();
      final dm = relay.dmChannel();
      final friend = relay.person('Friend');
      final seen = relay.message(dm, friend, 'already on screen');
      final unseen = relay.message(dm, friend, 'not yet');
      final release = relay.hold = Completer<void>();

      final wake = fixture.wake();
      await untilQueries(relay, 1);
      // The foreground app shows the message and records it.
      expect(
        await fixture.openStore().claim(
          relay.communityId,
          seen.id,
          DateTime.fromMillisecondsSinceEpoch(
            seen.createdAt * 1000,
            isUtc: true,
          ),
        ),
        isTrue,
      );
      release.complete();

      expect((await wake).notifications.map((n) => n.eventId), [unseen.id]);
    });

    test('a wake mid-run reruns once, and loses and repeats nothing', () async {
      final relay = fixture.addCommunity();
      final dm = relay.dmChannel();
      final friend = relay.person('Friend');
      final before = relay.message(dm, friend, 'before the fetch');
      final release = relay.hold = Completer<void>();
      var passes = 0;
      final runner = PushWakeRunner(() {
        passes++;
        return fixture.wake();
      });

      final first = runner.wake();
      await untilQueries(relay, 1);
      // A message arrives after the fetch started, and its wake finds a run
      // already going.
      final after = relay.message(dm, friend, 'after the fetch');
      final second = runner.wake();
      final third = runner.wake();
      release.complete();
      relay.hold = null;

      final results = await Future.wait([first, second, third]);

      expect(passes, 2);
      for (final result in results) {
        expect(result.notifications.map((n) => n.eventId), [
          after.id,
          before.id,
        ]);
      }
    });
  });
}
