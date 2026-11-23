import 'dart:convert';

import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/unread_badge/unread_badge_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/fake_relay.dart';

const _channel = '11111111-1111-4111-8111-111111111111';
const _added = '22222222-2222-4222-8222-222222222222';
const _third = '33333333-3333-4333-8333-333333333333';
const _fourth = '55555555-5555-4555-8555-555555555555';
const _dm = '44444444-4444-4444-8444-444444444444';

void main() {
  final binding = TestWidgetsFlutterBinding.ensureInitialized();

  test('while open, the channel list is read again soon after a live event '
      'says it changed, and only then', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    fakeAsync((async) {
      final relay = FakeRelay();
      var version = 0;
      NostrEvent roster(String channel, {bool withMe = true}) => relay.sign(
        kind: 39002,
        createdAt: 1700000000 + ++version,
        tags: [
          ['d', channel],
          if (withMe) ['p', relay.pubkey],
        ],
      );
      NostrEvent meta(String channel, String name, {bool archived = false}) =>
          relay.sign(
            kind: 39000,
            createdAt: 1700000000 + ++version,
            tags: [
              ['d', channel],
              ['name', name],
              ['t', 'stream'],
              ['public'],
              if (archived) ['archived'],
            ],
          );
      NostrEvent notice(int kind, String channel) => relay.sign(
        kind: kind,
        tags: [
          ['p', relay.pubkey],
          ['h', channel],
        ],
      );
      NostrEvent system(String type) => relay.sign(
        kind: EventKind.systemMessage,
        content: jsonEncode({'type': type}),
        tags: [
          ['h', _channel],
        ],
      );
      relay.seed([roster(_channel), meta(_channel, 'general')]);
      final container = ProviderContainer(
        overrides: [
          ...relay.overrides,
          savedPrefsProvider.overrideWithValue(prefs),
        ],
      );
      addTearDown(container.dispose);
      final channels = container.listen(channelsProvider, (_, _) {});
      int membershipReads() => relay.queries
          .expand((filters) => filters)
          .where(
            (f) =>
                f.containsKey('#p') &&
                f['kinds'].contains(39002) &&
                f['until'] == null,
          )
          .length;
      List<String> names() => [
        for (final channel in channels.read().requireValue) channel.name,
      ];
      void minutes(int n) => async.elapse(Duration(minutes: n));
      void seconds(int n) => async.elapse(Duration(seconds: n));

      async.elapse(const Duration(milliseconds: 50));
      expect(names(), ['general']);
      expect(membershipReads(), 1);

      // Nothing changed: nothing is read.
      minutes(5);
      expect(membershipReads(), 1);

      // Someone adds the reader to a channel. The list is read once the
      // notices go quiet for two seconds.
      relay.seed([roster(_added), meta(_added, 'added')]);
      relay.deliver(notice(EventKind.memberAddedNotification, _added));
      seconds(1);
      expect(membershipReads(), 1);
      seconds(2);
      expect(membershipReads(), 2);
      expect(names(), unorderedEquals(['general', 'added']));
      minutes(3);
      expect(membershipReads(), 2);

      // A listed channel is renamed.
      relay.seed([meta(_channel, 'renamed')]);
      relay.deliver(
        relay.sign(
          kind: EventKind.nip29EditMetadata,
          tags: [
            ['h', _channel],
            ['name', 'renamed'],
          ],
        ),
      );
      seconds(3);
      expect(membershipReads(), 3);
      expect(names(), unorderedEquals(['renamed', 'added']));

      // A member joining doesn't change the list; archiving it does.
      relay.deliver(system('member_joined'));
      minutes(2);
      expect(membershipReads(), 3);
      relay.deliver(system('channel_archived'));
      seconds(3);
      expect(membershipReads(), 4);
      // So does the relay archiving an idle ephemeral channel. Coming just
      // after a read, it is read once ten seconds have passed since that one.
      relay.deliver(system('channel_auto_archived'));
      seconds(3);
      expect(membershipReads(), 4);
      seconds(7);
      expect(membershipReads(), 5);
      minutes(1);

      // A steady stream of changes is read ten seconds after the first, and
      // then no more often than every ten seconds.
      for (var i = 0; i < 12; i++) {
        relay.deliver(system('topic_changed'));
        seconds(1);
      }
      expect(membershipReads(), 6);
      seconds(7);
      expect(membershipReads(), 6);
      seconds(2);
      expect(membershipReads(), 7);
      minutes(1);

      // A read that fails is retried with backoff, but still no sooner than
      // ten seconds after it.
      expect(names(), unorderedEquals(['renamed', 'added']));
      relay.failQuery(
        (f) => f.containsKey('#p') && (f['kinds'] as List).contains(39002),
      );
      // Like the relay, keep only the latest metadata for the channel.
      relay.stored.removeWhere(
        (e) => e.kind == 39000 && e.getTagValue('d') == _channel,
      );
      relay.seed([meta(_channel, 'retried')]);
      relay.deliver(system('topic_changed'));
      seconds(3);
      expect(names(), unorderedEquals(['renamed', 'added']));
      seconds(7);
      expect(names(), unorderedEquals(['renamed', 'added']));
      seconds(3);
      expect(names(), unorderedEquals(['retried', 'added']));
      final readsAfterRetry = membershipReads();
      minutes(10);
      expect(membershipReads(), readsAfterRetry);

      // A burst of changes costs one read.
      // The relay replaces the channel's roster with one without the reader.
      relay.stored.removeWhere(
        (e) => e.kind == 39002 && e.getTagValue('d') == _added,
      );
      relay.seed([roster(_added, withMe: false)]);
      relay.deliver(notice(EventKind.memberRemovedNotification, _added));
      relay.deliver(system('topic_changed'));
      seconds(3);
      expect(membershipReads(), readsAfterRetry + 1);
      expect(names(), ['retried']);
      minutes(3);
      expect(membershipReads(), readsAfterRetry + 1);

      // Another device hides a DM. The read that follows doesn't get the
      // hidden DMs, so it keeps the DM shown and is retried.
      relay.seed([
        roster(_dm),
        relay.sign(
          kind: 39000,
          createdAt: 1700000000 + ++version,
          tags: [
            ['d', _dm],
            ['name', 'dm'],
            ['t', 'dm'],
            ['private'],
            ['p', relay.pubkey],
          ],
        ),
      ]);
      relay.deliver(notice(EventKind.memberAddedNotification, _dm));
      seconds(3);
      expect(names(), unorderedEquals(['retried', 'dm']));
      minutes(1);
      relay.refuseReq(
        (f) =>
            (f['kinds'] as List).length == 1 &&
            (f['kinds'] as List).single == EventKind.dmVisibility,
        'error: unavailable',
      );
      relay.deliver(
        relay.sign(
          kind: EventKind.dmVisibility,
          tags: [
            ['p', relay.pubkey],
            ['h', _dm],
          ],
        ),
      );
      seconds(3);
      expect(names(), unorderedEquals(['retried', 'dm']));
      seconds(10);
      expect(names(), ['retried']);
      final readsAfterHide = membershipReads();
      minutes(10);
      expect(membershipReads(), readsAfterHide);

      // The list is rebuilt in place, as creating a Solo channel does, and
      // the read that follows fails. It is retried.
      relay.seed([roster(_third), meta(_third, 'third')]);
      relay.failQuery(
        (f) => f.containsKey('#p') && (f['kinds'] as List).contains(39002),
      );
      container.invalidate(channelsProvider);
      seconds(1);
      expect(names(), ['retried']);
      seconds(10);
      expect(names(), unorderedEquals(['retried', 'third']));
      final readsAfterRebuild = membershipReads();
      minutes(10);
      expect(membershipReads(), readsAfterRebuild);

      // A slow read is still running when the next change comes due. The
      // change waits for the read to finish and then for the 10s gap, not
      // just for the read. Each of the read's queries takes seven seconds.
      var slowQuery = relay.holdQuery();
      relay.deliver(system('topic_changed'));
      seconds(3);
      final slowRead = membershipReads();
      seconds(4);
      relay.deliver(system('topic_changed'));
      seconds(2);
      slowQuery.complete();
      slowQuery = relay.holdQuery();
      seconds(4);
      slowQuery.complete();
      seconds(1);
      expect(membershipReads(), slowRead);
      seconds(9);
      expect(membershipReads(), slowRead + 1);
      minutes(10);
      expect(membershipReads(), slowRead + 1);

      // A reconnect read fails, and its retry comes due while a slow read
      // after another reconnect is running. That read covers every change,
      // so the backoff starts over: the next failure is retried after the
      // first backoff step again, not a doubled one.
      bool isMembership(Map<String, dynamic> f) =>
          f.containsKey('#p') && (f['kinds'] as List).contains(39002);
      relay
        ..failQuery(isMembership)
        ..drop();
      seconds(1);
      final slowReconnect = relay.holdQuery();
      relay.drop();
      seconds(7);
      slowReconnect.complete();
      minutes(10);
      final readsAfterCovered = membershipReads();
      relay
        ..failQuery(isMembership)
        ..drop();
      while (membershipReads() == readsAfterCovered) {
        async.elapse(const Duration(milliseconds: 100));
      }
      // The first step is 3.75-6.25s with jitter; a doubled one is 7.5s or
      // more.
      async.elapse(const Duration(milliseconds: 6800));
      expect(membershipReads(), readsAfterCovered + 2);
      minutes(10);
      expect(membershipReads(), readsAfterCovered + 2);

      // On a cold load with no saved list, as after a fresh sign-in, the
      // read doesn't get the hidden DMs. It keeps the DM shown and is
      // retried.
      final fresh = FakeRelay();
      fresh
        ..seed([
          fresh.sign(
            kind: 39002,
            tags: [
              ['d', _dm],
              ['p', fresh.pubkey],
            ],
          ),
          fresh.sign(
            kind: 39000,
            tags: [
              ['d', _dm],
              ['name', 'dm'],
              ['t', 'dm'],
              ['private'],
              ['p', fresh.pubkey],
            ],
          ),
          fresh.sign(
            kind: EventKind.dmVisibility,
            tags: [
              ['p', fresh.pubkey],
              ['h', _dm],
            ],
          ),
        ])
        ..refuseReq(
          (f) =>
              (f['kinds'] as List).length == 1 &&
              (f['kinds'] as List).single == EventKind.dmVisibility,
          'error: unavailable',
        );
      final cold = ProviderContainer(
        overrides: [
          ...fresh.overrides,
          savedPrefsProvider.overrideWithValue(prefs),
        ],
      );
      addTearDown(cold.dispose);
      final coldChannels = cold.listen(channelsProvider, (_, _) {});
      List<String> coldNames() => [
        for (final channel in coldChannels.read().requireValue) channel.name,
      ];
      int coldReads() => fresh.queries
          .expand((filters) => filters)
          .where((f) => f.containsKey('#p') && f['kinds'].contains(39002))
          .length;
      seconds(1);
      expect(coldNames(), ['dm']);
      seconds(10);
      expect(coldNames(), isEmpty);
      final coldReadsAfterRetry = coldReads();
      minutes(10);
      expect(coldReads(), coldReadsAfterRetry);
    });
  });

  for (final refused in ['notices', 'live chunk']) {
    test('refused $refused are retried with backoff, then the list catches '
        'up once', () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      fakeAsync((async) {
        final relay = FakeRelay();
        var version = 0;
        NostrEvent roster(String channel, {bool withMe = true}) => relay.sign(
          kind: 39002,
          createdAt: 1700000000 + ++version,
          tags: [
            ['d', channel],
            if (withMe) ['p', relay.pubkey],
          ],
        );
        NostrEvent meta(String channel, String name) => relay.sign(
          kind: 39000,
          createdAt: 1700000000 + ++version,
          tags: [
            ['d', channel],
            ['name', name],
            ['t', 'stream'],
            ['public'],
          ],
        );
        // Like the relay, keep only the latest addressable event.
        void replace(NostrEvent event) {
          relay.stored.removeWhere(
            (e) =>
                e.kind == event.kind &&
                e.getTagValue('d') == event.getTagValue('d'),
          );
          relay.seed([event]);
        }

        void notice(int kind, String channel) => relay.deliver(
          relay.sign(
            kind: kind,
            tags: [
              ['p', relay.pubkey],
              ['h', channel],
            ],
          ),
        );
        void rename(String name) {
          replace(meta(_channel, name));
          relay.deliver(
            relay.sign(
              kind: EventKind.nip29EditMetadata,
              tags: [
                ['h', _channel],
                ['name', name],
              ],
            ),
          );
        }

        bool isNotices(Map<String, dynamic> f) =>
            (f['kinds'] as List).contains(EventKind.memberAddedNotification);
        bool isChannelChunk(Map<String, dynamic> f) =>
            f['limit'] == 0 &&
            f['#h'] is List &&
            (f['#h'] as List).length == 1 &&
            (f['#h'] as List).single == _channel;
        final refusedReq = refused == 'notices' ? isNotices : isChannelChunk;
        int reqs(bool Function(Map<String, dynamic>) match) => relay.reqs
            .where((m) => m.skip(2).cast<Map<String, dynamic>>().any(match))
            .length;
        // The relay is over its subscription limit at launch and refuses
        // the subscription twice.
        const full = 'error: too many subscriptions';
        relay
          ..refuseReq(refusedReq, full)
          ..refuseReq(refusedReq, full)
          ..seed([roster(_channel), meta(_channel, 'general')]);
        final container = ProviderContainer(
          overrides: [
            ...relay.overrides,
            savedPrefsProvider.overrideWithValue(prefs),
          ],
        );
        addTearDown(container.dispose);
        final channels = container.listen(channelsProvider, (_, _) {});
        int membershipReads() => relay.queries
            .expand((filters) => filters)
            .where(
              (f) =>
                  f.containsKey('#p') &&
                  f['kinds'].contains(39002) &&
                  f['until'] == null,
            )
            .length;
        List<String> names() => [
          for (final channel in channels.read().requireValue) channel.name,
        ];

        async.elapse(const Duration(milliseconds: 50));
        expect(names(), ['general']);
        expect(membershipReads(), 1);

        // While it is down, the list changes. The event has nowhere to go.
        if (refused == 'notices') {
          replace(roster(_added));
          relay.seed([meta(_added, 'added')]);
          notice(EventKind.memberAddedNotification, _added);
        } else {
          rename('renamed');
        }
        final caughtUp = refused == 'notices'
            ? ['general', 'added']
            : ['renamed'];

        // It is retried with backoff (5s, then 10s, each +/-25%) and admitted
        // by 19s. What it missed is not replayed, so once it has stayed up
        // for 2s the list is read again to catch up.
        async.elapse(const Duration(seconds: 3));
        expect(reqs(refusedReq), 1);
        async.elapse(const Duration(seconds: 7));
        expect(reqs(refusedReq), 2);
        expect(membershipReads(), 1);
        async.elapse(const Duration(seconds: 11));
        expect(reqs(refusedReq), 3);
        expect(membershipReads(), 2);
        expect(names(), unorderedEquals(caughtUp));
        async.elapse(const Duration(minutes: 5));
        expect(membershipReads(), 2);
        expect(reqs(refusedReq), 3);

        // Having stayed up, it starts its backoff over: when the relay
        // drops it, it is reopened within about 5s and catches up again.
        relay.closeLive(refusedReq, full);
        async.elapse(const Duration(seconds: 7));
        expect(reqs(refusedReq), 4);
        async.elapse(const Duration(seconds: 2));
        expect(membershipReads(), 3);
        async.elapse(const Duration(minutes: 1));

        // It is live.
        if (refused == 'notices') {
          replace(roster(_added, withMe: false));
          notice(EventKind.memberRemovedNotification, _added);
        } else {
          rename('live');
        }
        async.elapse(const Duration(seconds: 3));
        expect(membershipReads(), 4);
        expect(names(), refused == 'notices' ? ['general'] : ['live']);
        async.elapse(const Duration(minutes: 1));

        // The relay closes it for a reason that may clear, and the relay
        // session reopens it itself a second later. What changed meanwhile
        // is not replayed, so the list catches up once.
        relay.closeLive(refusedReq, 'error: relay busy');
        if (refused == 'notices') {
          replace(roster(_added));
          notice(EventKind.memberAddedNotification, _added);
        } else {
          rename('reopened');
        }
        async.elapse(const Duration(seconds: 2));
        expect(reqs(refusedReq), 5);
        expect(membershipReads(), 4);
        async.elapse(const Duration(seconds: 2));
        expect(membershipReads(), 5);
        expect(
          names(),
          unorderedEquals(
            refused == 'notices' ? ['general', 'added'] : ['reopened'],
          ),
        );
        async.elapse(const Duration(minutes: 5));
        expect(membershipReads(), 5);

        // One reopened and then closed again before its read is due counts
        // only once it stays up: here, after two more refusals (1s, 2s, 4s).
        relay.closeLive(refusedReq, 'error: relay busy');
        async.elapse(const Duration(milliseconds: 1500));
        expect(reqs(refusedReq), 6);
        relay
          ..closeLive(refusedReq, 'error: relay busy')
          ..refuseReq(refusedReq, 'error: relay busy')
          ..refuseReq(refusedReq, 'error: relay busy');
        async.elapse(const Duration(milliseconds: 6500));
        expect(reqs(refusedReq), 8);
        expect(membershipReads(), 5);
        async.elapse(const Duration(seconds: 5));
        expect(reqs(refusedReq), 9);
        expect(membershipReads(), 6);
        async.elapse(const Duration(minutes: 5));
        expect(membershipReads(), 6);

        // Dropped as over the limit, it is retried after about 5s, but the
        // relay now refuses it for good. That can't clear on this
        // connection, so it is left alone.
        relay
          ..closeLive(refusedReq, full)
          ..refuseReq(refusedReq, 'restricted: not allowed');
        async.elapse(const Duration(seconds: 7));
        expect(reqs(refusedReq), 10);
        async.elapse(const Duration(minutes: 10));
        expect(reqs(refusedReq), 10);
        expect(membershipReads(), 6);

        // A new connection tries it again, and reads the list.
        void lifecycle(AppLifecycleState state) =>
            binding.handleAppLifecycleStateChanged(state);
        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.hidden);
        lifecycle(AppLifecycleState.paused);
        async.elapse(const Duration(minutes: 1));
        lifecycle(AppLifecycleState.hidden);
        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.resumed);
        async.elapse(const Duration(seconds: 1));
        expect(reqs(refusedReq), 11);
        expect(membershipReads(), 7);
      });
    });
  }

  test(
    'the channel list reconciles once per resume and stays quiet while away',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      fakeAsync((async) {
        var now = DateTime(2026, 10, 20, 12);
        final relay = FakeRelay(now: () => now);
        relay.seed([
          relay.sign(
            kind: 39002,
            tags: [
              ['d', _channel],
              ['p', relay.pubkey],
            ],
          ),
          relay.sign(
            kind: 39000,
            tags: [
              ['d', _channel],
              ['name', 'general'],
              ['t', 'stream'],
              ['public'],
            ],
          ),
        ]);
        final container = ProviderContainer(
          overrides: [
            ...relay.overrides,
            savedPrefsProvider.overrideWithValue(prefs),
          ],
        );
        addTearDown(container.dispose);
        final channels = container.listen(channelsProvider, (_, _) {});
        // A membership read is paginated; count each read's first page.
        int membershipReads() => relay.queries
            .expand((filters) => filters)
            .where(
              (f) =>
                  f.containsKey('#p') &&
                  f['kinds'].contains(39002) &&
                  f['until'] == null,
            )
            .length;
        void lifecycle(AppLifecycleState state) =>
            binding.handleAppLifecycleStateChanged(state);
        void settle() => async.elapse(const Duration(milliseconds: 50));

        settle();
        expect(channels.read().requireValue.single.name, 'general');
        expect(membershipReads(), 1);

        // Control Center, Notification Center or a system alert passes
        // through inactive without leaving the foreground: not a resume.
        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.resumed);
        settle();
        expect(membershipReads(), 1);

        // Backgrounded while the session is still connected through its
        // grace period: nothing is read until the app returns.
        async.elapse(const Duration(seconds: 58));
        expect(membershipReads(), 1);
        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.hidden);
        lifecycle(AppLifecycleState.paused);
        async.elapse(const Duration(minutes: 3));
        expect(
          container.read(relaySessionProvider).status,
          isNot(SessionStatus.connected),
        );
        expect(membershipReads(), 1);

        lifecycle(AppLifecycleState.hidden);
        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.resumed);
        settle();
        expect(
          container.read(relaySessionProvider).status,
          SessionStatus.connected,
        );
        expect(membershipReads(), 2);

        // The OS suspends the app, so the session's pause timer never fires
        // and it still believes it is connected when the app resumes. It
        // reconnects; the list reconciles once, after AUTH, not before too.
        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.hidden);
        lifecycle(AppLifecycleState.paused);
        now = now.add(const Duration(minutes: 10));
        final auth = relay.holdAuth();
        lifecycle(AppLifecycleState.hidden);
        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.resumed);
        settle();
        expect(membershipReads(), 2);
        auth.complete();
        settle();
        expect(
          container.read(relaySessionProvider).status,
          SessionStatus.connected,
        );
        expect(membershipReads(), 3);
        expect(relay.reqsBeforeAuth, isEmpty);
        expect(channels.read().requireValue.single.name, 'general');

        // A read after a reconnect or a resume that fails is retried, so
        // what changed while away still shows up.
        List<String> names() => [
          for (final channel in channels.read().requireValue) channel.name,
        ];
        void addChannel(String id, String name) => relay.seed([
          relay.sign(
            kind: 39002,
            tags: [
              ['d', id],
              ['p', relay.pubkey],
            ],
          ),
          relay.sign(
            kind: 39000,
            tags: [
              ['d', id],
              ['name', name],
              ['t', 'stream'],
              ['public'],
            ],
          ),
        ]);
        void failNextMembershipRead() => relay.failQuery(
          (f) => f.containsKey('#p') && (f['kinds'] as List).contains(39002),
        );

        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.hidden);
        lifecycle(AppLifecycleState.paused);
        async.elapse(const Duration(minutes: 3));
        addChannel(_added, 'added');
        failNextMembershipRead();
        lifecycle(AppLifecycleState.hidden);
        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.resumed);
        settle();
        expect(membershipReads(), 4);
        expect(names(), ['general']);
        async.elapse(const Duration(seconds: 7));
        expect(membershipReads(), 5);
        expect(names(), unorderedEquals(['general', 'added']));
        async.elapse(const Duration(minutes: 1));

        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.hidden);
        lifecycle(AppLifecycleState.paused);
        // Back within the session's 5s grace, so it stays connected.
        async.elapse(const Duration(seconds: 1));
        addChannel(_third, 'third');
        failNextMembershipRead();
        lifecycle(AppLifecycleState.hidden);
        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.resumed);
        settle();
        expect(
          container.read(relaySessionProvider).status,
          SessionStatus.connected,
        );
        expect(membershipReads(), 6);
        expect(names(), unorderedEquals(['general', 'added']));
        async.elapse(const Duration(seconds: 7));
        expect(membershipReads(), 7);
        expect(names(), unorderedEquals(['general', 'added', 'third']));
        async.elapse(const Duration(minutes: 10));
        expect(membershipReads(), 7);

        // The retry comes due while Control Center is open. That is still the
        // foreground and no resume follows it, so the retry is not left to one.
        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.hidden);
        lifecycle(AppLifecycleState.paused);
        async.elapse(const Duration(seconds: 1));
        addChannel(_fourth, 'fourth');
        failNextMembershipRead();
        lifecycle(AppLifecycleState.hidden);
        lifecycle(AppLifecycleState.inactive);
        lifecycle(AppLifecycleState.resumed);
        settle();
        expect(membershipReads(), 8);
        lifecycle(AppLifecycleState.inactive);
        async.elapse(const Duration(seconds: 7));
        lifecycle(AppLifecycleState.resumed);
        settle();
        expect(membershipReads(), 9);
        expect(
          names(),
          unorderedEquals(['general', 'added', 'third', 'fourth']),
        );
        async.elapse(const Duration(minutes: 10));
        expect(membershipReads(), 9);

        // The connection drops and a message lands in a quiet channel. The
        // unread catch-up after the reconnect fails, and is retried until the
        // channel shows as unread.
        int unread() => container.read(unreadBadgeProvider).generalUnreadCount;
        bool isCatchUp(Map<String, dynamic> f) =>
            f['since'] != null &&
            (f['kinds'] as List).contains(EventKind.streamMessage);
        int catchUps() =>
            relay.queries.where((filters) => filters.any(isCatchUp)).length;
        expect(unread(), 0);
        final catchUpsBefore = catchUps();
        relay.failQuery(isCatchUp, body: '{"error":"query timed out"}');
        relay.drop();
        relay.seed([
          relay.sign(
            kind: EventKind.streamMessage,
            content: 'while down',
            createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
            tags: [
              ['h', _added],
            ],
          ),
        ]);
        async.elapse(const Duration(seconds: 2));
        expect(catchUps(), catchUpsBefore + 1);
        expect(unread(), 0);
        async.elapse(const Duration(seconds: 15));
        expect(catchUps(), catchUpsBefore + 2);
        expect(unread(), 1);
        async.elapse(const Duration(minutes: 10));
        expect(catchUps(), catchUpsBefore + 2);

        // Again, but the batch is only partly unavailable: it falls back to
        // per-channel reads, and only one channel's read fails. That channel
        // is caught up on the retry.
        relay
          ..failQuery(isCatchUp)
          ..refuseReq(
            (f) =>
                isCatchUp(f) &&
                f['limit'] != 0 &&
                (f['#h'] as List? ?? const []).contains(_channel),
            'error: unavailable',
          )
          ..drop()
          ..seed([
            relay.sign(
              kind: EventKind.streamMessage,
              content: 'while down again',
              createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
              tags: [
                ['h', _channel],
              ],
            ),
          ]);
        async.elapse(const Duration(seconds: 2));
        expect(catchUps(), catchUpsBefore + 3);
        expect(unread(), 1);
        async.elapse(const Duration(seconds: 15));
        expect(catchUps(), catchUpsBefore + 4);
        expect(unread(), 2);
        async.elapse(const Duration(minutes: 10));
        expect(catchUps(), catchUpsBefore + 4);
      });
    },
  );
}
