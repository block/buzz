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

  test('while open, the channel list is read again only after a live event '
      'says it changed', () async {
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

      async.elapse(const Duration(milliseconds: 50));
      expect(names(), ['general']);
      expect(membershipReads(), 1);

      // Nothing changed: the backstop ticks send nothing.
      minutes(5);
      expect(membershipReads(), 1);

      // Someone adds the reader to a channel.
      relay.seed([roster(_added), meta(_added, 'added')]);
      relay.deliver(notice(EventKind.memberAddedNotification, _added));
      minutes(1);
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
      minutes(1);
      expect(membershipReads(), 3);
      expect(names(), unorderedEquals(['renamed', 'added']));

      // A member joining doesn't change the list; archiving it does.
      relay.deliver(system('member_joined'));
      minutes(2);
      expect(membershipReads(), 3);
      relay.deliver(system('channel_archived'));
      minutes(1);
      expect(membershipReads(), 4);
      // So does the relay archiving an idle ephemeral channel.
      relay.deliver(system('channel_auto_archived'));
      minutes(1);
      expect(membershipReads(), 5);

      // Several changes within a minute cost one read.
      // The relay replaces the channel's roster with one without the reader.
      relay.stored.removeWhere(
        (e) => e.kind == 39002 && e.getTagValue('d') == _added,
      );
      relay.seed([roster(_added, withMe: false)]);
      relay.deliver(notice(EventKind.memberRemovedNotification, _added));
      relay.deliver(system('topic_changed'));
      minutes(1);
      expect(membershipReads(), 6);
      expect(names(), ['renamed']);
      minutes(3);
      expect(membershipReads(), 6);
    });
  });

  test('refused live subscriptions are retried, then the list catches up '
      'once', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    fakeAsync((async) {
      final relay = FakeRelay();
      var version = 0;
      NostrEvent roster(String channel) => relay.sign(
        kind: 39002,
        createdAt: 1700000000 + ++version,
        tags: [
          ['d', channel],
          ['p', relay.pubkey],
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
      bool isNotices(Map<String, dynamic> f) =>
          (f['kinds'] as List).contains(EventKind.memberAddedNotification);
      bool isChunk(Map<String, dynamic> f) =>
          f.containsKey('#h') && f['limit'] == 0;
      int reqs(bool Function(Map<String, dynamic>) match) => relay.reqs
          .where((m) => m.skip(2).cast<Map<String, dynamic>>().any(match))
          .length;
      // The relay is over its subscription limit at launch: it refuses the
      // notices twice and the channel's live chunk once.
      const full = 'error: too many subscriptions';
      relay
        ..refuseReq(isNotices, full)
        ..refuseReq(isNotices, full)
        ..refuseReq(isChunk, full)
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

      // While both are down, the reader is added to a channel. The notice
      // has nowhere to go.
      relay.seed([roster(_added), meta(_added, 'added')]);
      relay.deliver(
        relay.sign(
          kind: EventKind.memberAddedNotification,
          tags: [
            ['p', relay.pubkey],
            ['h', _added],
          ],
        ),
      );

      // Both are retried with backoff and admitted within half a minute.
      // Reopening them sends no list read by itself ...
      async.elapse(const Duration(seconds: 30));
      expect(reqs(isNotices), 3);
      expect(reqs(isChunk), 2);
      expect(membershipReads(), 1);
      // ... but the next tick reads once to catch up on what they missed.
      async.elapse(const Duration(seconds: 31));
      expect(membershipReads(), 2);
      expect(names(), unorderedEquals(['general', 'added']));
      async.elapse(const Duration(minutes: 3));
      expect(membershipReads(), 2);
      expect(reqs(isNotices), 3);

      // Both are live again: a rename on the reopened chunk is picked up.
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
      async.elapse(const Duration(minutes: 1));
      expect(membershipReads(), 3);
      expect(names(), unorderedEquals(['renamed', 'added']));
    });
  });

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

        // Backgrounded just before a backstop tick, while the session is
        // still connected through its grace period: the 60s backstop sends
        // nothing until the app returns.
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
