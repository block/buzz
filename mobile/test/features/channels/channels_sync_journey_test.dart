import 'dart:convert';

import 'package:buzz/features/channels/channels_provider.dart';
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

      // A read that fails is retried with backoff.
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
      });
    },
  );
}
