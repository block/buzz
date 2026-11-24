import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/push/push_wake_resolver.dart';

import 'package:buzz/shared/push/push_wake_runner.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:buzz/shared/utils/string_utils.dart';
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

  // The message fetches a relay saw, without the name lookups after them.
  Iterable<List<Map<String, dynamic>>> messageQueries(FakePushRelay relay) =>
      relay.queries.where((query) => query.first.containsKey('since'));

  // Records a message as shown [age] ago, as an earlier wake would have.
  Future<void> shownAgo(FakePushRelay relay, Duration age) async {
    expect(
      await fixture.openStore().claim(
        relay.communityId,
        age.inSeconds.toRadixString(16).padLeft(64, '0'),
        pushWakeClock.subtract(age),
      ),
      isTrue,
    );
  }

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

    // Messages are read from the writer, so one that just arrived is not
    // missed to a lagging replica. Names can come from any replica.
    for (final filter in relay.queries.expand((query) => query)) {
      expect(
        filter['consistency'],
        filter.containsKey('since') ? 'strong' : isNull,
        reason: '$filter',
      );
    }
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

    // Tapping a reply opens the thread of the message it answers, as in the
    // inbox: a reply to a reply opens its direct parent's thread, and a
    // broadcast reply is a top-level message with no thread.
    List<List<String>> replyTo(NostrEvent root, [NostrEvent? parent]) => [
      ['e', root.id, '', 'root'],
      ['e', (parent ?? root).id, '', 'reply'],
    ];
    final start = relay.message(dm, friend, 'thread start');
    final reply = relay.message(
      dm,
      friend,
      'first reply',
      extraTags: replyTo(start),
    );
    final nested = relay.message(
      dm,
      friend,
      'reply to the reply',
      extraTags: replyTo(start, reply),
    );
    final broadcast = relay.message(
      dm,
      friend,
      'also for the channel',
      extraTags: [
        ...replyTo(start, reply),
        ['broadcast', '1'],
      ],
    );
    expect(
      {
        for (final n in (await fixture.wake()).notifications)
          n.eventId: n.target.threadRootId,
      },
      {
        start.id: null,
        reply.id: start.id,
        nested.id: reply.id,
        broadcast.id: null,
      },
    );

    // Names come from signed events only. A profile or channel name that
    // fails verification leaves the short key and no channel name, and the
    // message still shows.
    final stranger = relay.person('Impostor', forged: true);
    final lookalike = relay.channel(forged: true);
    final forged = relay.message(
      lookalike,
      stranger,
      'trust me',
      mention: relay.me,
    );
    final unnamed = (await fixture.wake()).notifications.single;
    expect(unnamed.eventId, forged.id);
    expect(unnamed.title, shortPubkey(stranger.pubkey));
    expect(unnamed.conversationTitle, isNull);
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

    // After a long quiet spell, every message since the last shown one
    // notifies, however far apart they arrived.
    final quiet = fixture.addCommunity();
    await shownAgo(quiet, const Duration(hours: 2));
    final quietDm = quiet.dmChannel();
    final quietFriend = quiet.person('Friend');
    final earlier = quiet.message(
      quietDm,
      quietFriend,
      'while you were away',
      age: const Duration(minutes: 50),
    );
    final latest = quiet.message(quietDm, quietFriend, 'and now');
    final caughtUp = await fixture.wake();
    expect(caughtUp.incomplete, isFalse);
    expect(
      caughtUp.notifications
          .where((n) => n.target.communityId == quiet.communityId)
          .map((n) => n.eventId),
      [latest.id, earlier.id],
    );

    // Messages that can't notify can fill a page of the fetch, and an
    // eligible one behind them must still show: here a DM behind messages
    // already shown, and a mention behind mentions in a muted channel.
    final crowded = fixture.addCommunity();
    final crowdedDm = crowded.dmChannel();
    final mutedDm = crowded.dmChannel();
    final crowdedRoom = crowded.channel();
    crowded.mute(mutedDm);
    final chatty = crowded.person('Chatty');
    const buriedAge = Duration(minutes: 12);
    final buriedDm = crowded.message(
      crowdedDm,
      chatty,
      'buried',
      age: buriedAge,
    );
    final buriedMention = crowded.message(
      crowdedRoom,
      chatty,
      'buried mention',
      mention: crowded.me,
      age: buriedAge,
    );
    for (var i = 0; i < pushWakeMessagePageSize + 5; i++) {
      final seen = crowded.message(crowdedDm, chatty, 'seen $i');
      expect(
        await fixture.openStore().claim(
          crowded.communityId,
          seen.id,
          DateTime.fromMillisecondsSinceEpoch(
            seen.createdAt * 1000,
            isUtc: true,
          ),
        ),
        isTrue,
      );
      crowded.message(mutedDm, chatty, 'muted $i', mention: crowded.me);
    }
    final dug = await fixture.wake();
    expect(dug.incomplete, isFalse);
    expect(
      dug.notifications
          .where((n) => n.target.communityId == crowded.communityId)
          .map((n) => n.eventId),
      unorderedEquals([buriedDm.id, buriedMention.id]),
    );
    expect(crowded.queries.first.every((f) => f['until'] == null), isTrue);
    expect(crowded.queries[1].every((f) => f['until'] != null), isTrue);
    expect((await fixture.wake()).notifications, isEmpty);

    // Subscriptions overlap: a DM that mentions me answers the DM filter and
    // also matches the mention filter. Being older than the room's mentions,
    // it must not move the mention filter's next page past them.
    final overlap = fixture.addCommunity();
    final palDm = overlap.dmChannel();
    final noisyDm = overlap.dmChannel();
    final room = overlap.channel();
    overlap.mute(noisyDm);
    final pal = overlap.person('Pal');
    final expected = [
      overlap
          .message(
            palDm,
            pal,
            'dm mention',
            mention: overlap.me,
            age: const Duration(minutes: 13),
          )
          .id,
      for (final minutes in [12, 11, 10])
        overlap
            .message(
              room,
              pal,
              'mention $minutes',
              mention: overlap.me,
              age: Duration(minutes: minutes),
            )
            .id,
    ];
    for (var i = 0; i < pushWakeMessagePageSize + 5; i++) {
      overlap.message(noisyDm, pal, 'muted $i', mention: overlap.me);
    }
    final untangled = await fixture.wake();
    expect(untangled.incomplete, isFalse);
    expect(
      untangled.notifications
          .where((n) => n.target.communityId == overlap.communityId)
          .map((n) => n.eventId),
      unorderedEquals(expected),
    );
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

    // Storage fails midway through one community. The other community's
    // message and the one claimed before the failure still show, and the
    // failing community is reported so the next wake finishes it, even
    // when the message left over arrived long after the one claimed.
    final [healthy, _] = relays;
    // A community whose last shown message is old, so the one left over is
    // also far from the one claimed.
    final failing = fixture.addCommunity();
    final newer = healthy.message(
      healthy.dmChannel(),
      healthy.person('A'),
      'a',
    );
    await shownAgo(failing, const Duration(hours: 2));
    final failingDm = failing.dmChannel();
    final friend = failing.person('B');
    final claimed = failing.message(
      failingDm,
      friend,
      'older',
      age: const Duration(minutes: 40),
    );
    final skipped = failing.message(failingDm, friend, 'newer');
    final partial = await fixture.wake(
      store: FailingShownMessageStore(
        fixture.openStore(),
        failing.communityId,
        succeed: 1,
      ),
    );
    expect(partial.notifications.map((n) => n.eventId).toSet(), {
      newer.id,
      claimed.id,
    });
    expect(partial.failedCommunityIds, [failing.communityId]);
    expect((await fixture.wake()).notifications.map((n) => n.eventId), [
      skipped.id,
    ]);
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

      // A relay that answers with more than a wake will buffer, inflate or
      // verify costs only its own community, and its messages are still
      // there once it behaves.
      final hostile = fixture.addCommunity();
      final hostileMessage = hostile.message(
        hostile.dmChannel(),
        hostile.person('Friend'),
        'hostile',
      );
      final signed = jsonEncode(hostileMessage.toJson());
      final hostileAnswers = <String, http.Response Function()>{
        // Tiny on the wire, huge once inflated.
        'a gzip bomb': () => http.Response.bytes(
          gzip.encode(utf8.encode('[${' ' * (pushWakeMaxResponseBytes + 1)}]')),
          200,
          headers: {'content-type': 'application/json'},
        ),
        'an oversized body': () => http.Response(
          '[${' ' * (pushWakeMaxResponseBytes + 1)}]',
          200,
          headers: {'content-type': 'application/json'},
        ),
        'more events than asked for': () => http.Response(
          '[${List.filled(1000, signed).join(',')}]',
          200,
          headers: {'content-type': 'application/json'},
        ),
      };
      for (final MapEntry(:key, :value) in hostileAnswers.entries) {
        hostile.hostileResponse = value;
        final attempt = await fixture.wake(timeouts: timeouts);
        expect(attempt.failedCommunityIds, [hostile.communityId], reason: key);
        expect(attempt.notifications, isEmpty, reason: key);
      }
      hostile.hostileResponse = null;
      final recovered = await fixture.wake(timeouts: timeouts);
      expect(recovered.notifications.map((n) => n.eventId), [
        hostileMessage.id,
      ]);

      // A page too big to take is asked for again with half the events, and
      // the fetch carries on at that size.
      final heavy = fixture.addCommunity();
      final heavyDm = heavy.dmChannel();
      heavy.mute(heavyDm);
      final bulky = heavy.person('Friend');
      final small = heavy.message(
        heavy.channel(),
        bulky,
        'small',
        mention: heavy.me,
        age: const Duration(minutes: 12),
      );
      final bulk = 'x' * 90000;
      for (var i = 0; i < 60; i++) {
        heavy.message(heavyDm, bulky, bulk, mention: heavy.me);
      }
      final halved = await fixture.wake();
      expect(halved.incomplete, isFalse);
      expect(halved.notifications.map((n) => n.eventId), [small.id]);
      expect(
        [for (final query in messageQueries(heavy)) query.first['limit']],
        [pushWakeMessagePageSize, 25, 25, 25],
      );

      // A relay with more history than the page cap allows still shows the
      // newest messages, and leaves its community unfinished.
      final endless = fixture.addCommunity();
      final endlessDm = endless.dmChannel();
      endless.mute(endlessDm);
      final noisy = endless.person('Friend');
      for (
        var i = 0;
        i < pushWakeMessagePageSize * pushWakeMaxMessagePages;
        i++
      ) {
        endless.message(endlessDm, noisy, 'noise $i', mention: endless.me);
      }
      final newest = endless.message(
        endless.channel(),
        noisy,
        'newest',
        mention: endless.me,
      );
      final capped = await fixture.wake();
      expect(capped.failedCommunityIds, [endless.communityId]);
      expect(
        capped.notifications
            .where((n) => n.target.communityId == endless.communityId)
            .map((n) => n.eventId),
        [newest.id],
      );
      expect(messageQueries(endless), hasLength(pushWakeMaxMessagePages));

      // A later page failing leaves the community unfinished too, and the
      // next wake reads on from what was shown.
      final flaky = fixture.addCommunity();
      final flakyDm = flaky.dmChannel();
      flaky.mute(flakyDm);
      final flakyFriend = flaky.person('Friend');
      final older = flaky.message(
        flaky.channel(),
        flakyFriend,
        'older',
        mention: flaky.me,
        age: const Duration(minutes: 12),
      );
      for (var i = 0; i < pushWakeMessagePageSize + 5; i++) {
        flaky.message(flakyDm, flakyFriend, 'noise $i', mention: flaky.me);
      }
      final latest = flaky.message(
        flaky.channel(),
        flakyFriend,
        'latest',
        mention: flaky.me,
      );
      flaky.unreachableAfter = 1;
      final cut = await fixture.wake();
      expect(cut.failedCommunityIds, contains(flaky.communityId));
      expect(
        cut.notifications
            .where((n) => n.target.communityId == flaky.communityId)
            .map((n) => n.eventId),
        [latest.id],
      );
      flaky.unreachableAfter = null;
      final resumed = await fixture.wake();
      expect(
        resumed.notifications
            .where((n) => n.target.communityId == flaky.communityId)
            .map((n) => n.eventId),
        [older.id],
      );

      // A full page inside one second can't be paged by time: the next
      // request would answer the same page. The fetch stops there, leaving
      // the community unfinished, rather than spending its pages on repeats.
      final stuck = fixture.addCommunity();
      final stuckDm = stuck.dmChannel();
      stuck.mute(stuckDm);
      final stuckFriend = stuck.person('Friend');
      stuck.message(
        stuck.channel(),
        stuckFriend,
        'behind',
        mention: stuck.me,
        age: const Duration(minutes: 12),
      );
      final second = pushWakeClock.subtract(const Duration(minutes: 1));
      for (var i = 0; i < pushWakeMessagePageSize + 5; i++) {
        stuck.message(
          stuckDm,
          stuckFriend,
          'same second $i',
          mention: stuck.me,
          at: second,
        );
      }
      final repeated = await fixture.wake();
      expect(repeated.failedCommunityIds, contains(stuck.communityId));
      expect(messageQueries(stuck), hasLength(2));
    });

    test('with nothing found say so, so the caller can fall back', () async {
      final relay = fixture.addCommunity()..unreachable = true;
      relay.message(relay.dmChannel(), relay.person('Friend'), 'hi');

      final result = await fixture.wake(timeouts: timeouts);

      expect(result.notifications, isEmpty);
      expect(result.incomplete, isTrue);

      // The budget starts with the wake, so a community list that never
      // loads can't hold it past that either.
      final unloaded = await fixture
          .wake(
            timeouts: timeouts,
            loadCommunities: () => Completer<List<Community>>().future,
          )
          .timeout(const Duration(seconds: 2));
      expect(unloaded.notifications, isEmpty);
      expect(unloaded.interrupted, isTrue);

      // The runner passes that on, so the caller still falls back.
      final run = await PushWakeRunner(
        (budget) => fixture.wake(
          timeouts: PushWakeTimeouts(total: budget),
          loadCommunities: () => Completer<List<Community>>().future,
        ),
      ).wake(remaining: Duration.zero).timeout(const Duration(seconds: 2));
      expect(run.notifications, isEmpty);
      expect(run.incomplete, isTrue);
    });

    test('the whole wake stops at its total budget, claims included', () async {
      final relay = fixture.addCommunity()..silent = true;
      final first = relay
          .message(relay.dmChannel(), relay.person('Friend'), 'hi')
          .id;
      const patient = PushWakeTimeouts(
        request: Duration(seconds: 30),
        total: Duration(milliseconds: 300),
      );

      final watch = Stopwatch()..start();
      final result = await fixture.wake(timeouts: patient);

      expect(watch.elapsed, lessThan(const Duration(seconds: 2)));
      expect(result.failedCommunityIds, [relay.communityId]);

      // Slow storage and a slow name lookup spend the budget too. Messages
      // not yet claimed stay unclaimed for the next wake, and those already
      // claimed are returned, not lost.
      relay.silent = false;
      final friend = relay.person('Friend');
      final sent = [
        first,
        for (var i = 0; i < 3; i++)
          for (final channel in [relay.dmChannel()])
            for (var j = 0; j < 6; j++)
              relay.message(channel, friend, 'message $i $j').id,
      ];
      const tight = PushWakeTimeouts(
        request: Duration(seconds: 30),
        metadata: Duration(seconds: 30),
        total: Duration(milliseconds: 700),
      );
      final slowStore = SlowShownMessageStore(
        fixture.openStore(),
        const Duration(milliseconds: 100),
      );
      final stopped = Stopwatch()..start();
      final partial = await fixture.wake(store: slowStore, timeouts: tight);
      expect(stopped.elapsed, lessThan(const Duration(seconds: 2)));
      expect(partial.incomplete, isTrue);
      expect(partial.notifications, isNotEmpty);
      expect(partial.notifications.length, lessThan(sent.length));
      final rest = await fixture.wake();
      expect(rest.incomplete, isFalse);
      expect(
        [...partial.notifications, ...rest.notifications].map((n) => n.eventId),
        unorderedEquals(sent),
      );

      relay.slowProfiles = true;
      final late = relay.message(relay.dmChannel(), friend, 'late names');
      final lookup = Stopwatch()..start();
      final unnamed = await fixture.wake(timeouts: tight);
      expect(lookup.elapsed, lessThan(const Duration(seconds: 2)));
      expect(unnamed.notifications.map((n) => n.eventId), [late.id]);

      // Storage that never answers for one community holds back neither the
      // others nor what it claimed before it stalled, and the community is
      // left for the next wake.
      final healthy = fixture.addCommunity();
      final stalled = fixture.addCommunity();
      final healthyFriend = healthy.person('Friend');
      final stalledFriend = stalled.person('Friend');
      final healthyDm = healthy.dmChannel();
      final stalledDm = stalled.dmChannel();
      final fine = healthy.message(healthyDm, healthyFriend, 'fine');
      final claimedFirst = stalled.message(stalledDm, stalledFriend, 'first');
      stalled.message(stalledDm, stalledFriend, 'second');
      final hung = Stopwatch()..start();
      final cutShort = await fixture
          .wake(
            store: StalledShownMessageStore(
              fixture.openStore(),
              stalled.communityId,
              succeed: 1,
            ),
            timeouts: tight,
          )
          .timeout(const Duration(seconds: 3));
      expect(hung.elapsed, lessThan(const Duration(seconds: 2)));
      expect(cutShort.failedCommunityIds, [stalled.communityId]);
      expect(cutShort.notifications.map((n) => n.eventId).toSet(), {
        fine.id,
        claimedFirst.id,
      });

      // Once the budget is spent, nothing more is sent to a relay.
      final gate = Completer<void>();
      int queried() => fixture.relays.fold(0, (n, r) => n + r.queries.length);
      final before = queried();
      final gated = await fixture.wake(
        store: SlowShownMessageStore(
          fixture.openStore(),
          Duration.zero,
          newestGate: gate.future,
        ),
        timeouts: const PushWakeTimeouts(total: Duration(milliseconds: 300)),
      );
      expect(gated.notifications, isEmpty);
      gate.complete();
      await Future<void>.delayed(const Duration(milliseconds: 100));
      expect(queried(), before);
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
      final runner = PushWakeRunner((budget) {
        passes++;
        return fixture.wake(timeouts: PushWakeTimeouts(total: budget));
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

      // All passes share one budget: when the first pass leaves too little
      // of it, a wake mid-run does not start another pass. The run returns
      // what it found, and is incomplete, since the later wake's message was
      // never looked for and the caller must fall back for it.
      final short = PushWakeRunner(
        (budget) {
          passes++;
          return fixture.wake(timeouts: PushWakeTimeouts(total: budget));
        },
        budget: const Duration(seconds: 5),
        minimumPass: const Duration(seconds: 10),
      );
      final late = relay.message(dm, friend, 'during a slow pass');
      final slow = relay.hold = Completer<void>();
      final queries = relay.queries.length;
      final fourth = short.wake();
      await untilQueries(relay, queries + 1);
      final fifth = short.wake();
      slow.complete();
      relay.hold = null;

      for (final result in await Future.wait([fourth, fifth])) {
        expect(result.notifications.map((n) => n.eventId), [late.id]);
        expect(result.incomplete, isTrue);
      }
      expect(passes, 3);

      // A rerun that fails keeps what the first pass already claimed, and
      // reports the run incomplete, so an empty first pass still ends in the
      // generic notification.
      var failingPasses = 0;
      final failing = PushWakeRunner((budget) {
        if (++failingPasses > 1) throw StateError('storage unreadable');
        return fixture.wake(timeouts: PushWakeTimeouts(total: budget));
      });
      final kept = relay.message(dm, friend, 'before a failed rerun');
      final held = relay.hold = Completer<void>();
      final queriesBefore = relay.queries.length;
      final sixth = failing.wake();
      await untilQueries(relay, queriesBefore + 1);
      final seventh = failing.wake();
      held.complete();
      relay.hold = null;
      for (final result in await Future.wait([sixth, seventh])) {
        expect(result.notifications.map((n) => n.eventId), [kept.id]);
        expect(result.incomplete, isTrue);
      }
      expect(failingPasses, 2);

      failingPasses = 0;
      final emptyHeld = relay.hold = Completer<void>();
      final emptyBefore = relay.queries.length;
      final eighth = failing.wake();
      await untilQueries(relay, emptyBefore + 1);
      final ninth = failing.wake();
      emptyHeld.complete();
      relay.hold = null;
      for (final result in await Future.wait([eighth, ninth])) {
        expect(result.notifications, isEmpty);
        expect(result.incomplete, isTrue);
      }
      expect(failingPasses, 2);
    });
  });
}
