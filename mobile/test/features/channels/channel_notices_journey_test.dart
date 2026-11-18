import 'dart:convert';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/channel_messages_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:crypto/crypto.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/fake_relay.dart';

const _channelId = '11111111-2222-4333-8444-555555555555';

// History fixtures cross the fake network boundary; signing hundreds of
// fixtures adds no paging coverage. Auth and window bounds still use FakeRelay.
NostrEvent _message(FakeRelay relay, int second) => NostrEvent(
  id: '8${second.toRadixString(16).padLeft(63, '0')}',
  pubkey: relay.pubkey,
  kind: EventKind.streamMessage,
  content: 'Message $second',
  createdAt: second,
  sig: '0' * 128,
  tags: const [
    ['h', _channelId],
  ],
);

NostrEvent _notice(FakeRelay relay, int second, String topic) => NostrEvent(
  id: sha256.convert(utf8.encode('$second:$topic')).toString(),
  pubkey: relay.pubkey,
  kind: EventKind.systemMessage,
  content: jsonEncode({'type': 'topic_changed', 'topic': topic}),
  createdAt: second,
  sig: '0' * 128,
  tags: const [
    ['h', _channelId],
  ],
);

Iterable<Map<String, dynamic>> _noticeReads(FakeRelay relay) => relay.queries
    .expand((filters) => filters)
    .where((filter) => (filter['kinds'] as List).singleOrNull == 40099);

void main() {
  // The real notifier and session issue HTTP filters; only the network is
  // fake. The fixture applies since, the composite cursor and the limit, so
  // removing a production bound or skipping a range changes the result.
  for (final count in [0, 1, 500, 501]) {
    test('notice ranges page through $count boundary-second notices', () {
      fakeAsync((clock) {
        final relay = FakeRelay();
        final messages = [for (var i = 1; i <= 151; i++) _message(relay, i)];
        final boundary = [
          for (var i = 0; i < count; i++) _notice(relay, 102, 'Boundary $i'),
        ];
        final burst = [
          if (count == 501)
            for (var i = 0; i < 501; i++) _notice(relay, 103, 'Burst $i'),
        ];
        final middle = _notice(relay, 75, 'Middle');
        final oldest = _notice(relay, 0, 'Before the first message');
        relay.seed([...messages, ...burst, ...boundary, middle, oldest]);
        final container = ProviderContainer(overrides: relay.overrides);
        addTearDown(container.dispose);
        final subscription = container.listen(
          channelMessagesProvider(_channelId),
          (_, _) {},
        );
        addTearDown(subscription.close);
        void settle() => clock.elapse(const Duration(milliseconds: 50));
        List<NostrEvent> shown() => subscription.read().requireValue;
        void checkIds(Iterable<NostrEvent> expected) {
          expect(
            shown().map((event) => event.id),
            unorderedEquals(expected.map((event) => event.id)),
          );
          expect(
            shown().map((event) => event.id).toSet(),
            hasLength(shown().length),
          );
        }

        final notifier = container.read(
          channelMessagesProvider(_channelId).notifier,
        );
        void older() {
          var completed = false;
          notifier.fetchOlder().then((loaded) {
            expect(loaded, isTrue);
            completed = true;
          });
          settle();
          expect(completed, isTrue);
        }

        settle();
        expect(_noticeReads(relay).first['since'], 102);
        if (burst.isNotEmpty) {
          expect(_noticeReads(relay), hasLength(2));
          expect(_noticeReads(relay).last['since'], 102);
        }
        checkIds([
          ...messages.skip(101),
          ...burst,
          ...boundary.where(
            (notice) => notice.id.compareTo(messages[101].id) <= 0,
          ),
        ]);

        // A short OR empty bounded response must not finish all history.
        // The previous boundary's hidden same-second notices now show too.
        older();
        checkIds([...messages.skip(51), ...burst, ...boundary, middle]);
        expect(_noticeReads(relay).last['since'], 52);

        // This range has no notices. It still advances, without rereading
        // the newest range or abandoning the notices before message history.
        older();
        checkIds([...messages.skip(1), ...burst, ...boundary, middle]);
        expect(_noticeReads(relay).last['since'], 2);
        older();
        expect(_noticeReads(relay).last['since'], isNull);
        checkIds([...messages, ...burst, ...boundary, middle, oldest]);
        expect(notifier.reachedOldest, isTrue);
        final reads = relay.queries.length;
        notifier.fetchOlder().then((loaded) => expect(loaded, isFalse));
        settle();
        expect(relay.queries, hasLength(reads));
      });
    });
  }

  test('same-second message pages reveal cached notices without a gap', () {
    fakeAsync((clock) {
      final relay = FakeRelay();
      final messages = [
        for (var i = 1; i <= 120; i++)
          NostrEvent.fromJson({
            ..._message(relay, i).toJson(),
            'created_at': 50,
            'id': '${i.toRadixString(16).padLeft(2, '0')}${'0' * 62}',
          }),
      ];
      final notice = NostrEvent.fromJson({
        ..._notice(relay, 50, 'Between equal-second messages').toJson(),
        // Between the 50th (0x32) and 51st (0x33) message in relay order.
        'id': '328${'0' * 61}',
      });
      final beforeMessages = _notice(relay, 0, 'Before all messages');
      relay.seed([...messages, notice, beforeMessages]);
      final container = ProviderContainer(overrides: relay.overrides);
      addTearDown(container.dispose);
      final subscription = container.listen(
        channelMessagesProvider(_channelId),
        (_, _) {},
      );
      addTearDown(subscription.close);
      void settle() => clock.elapse(const Duration(milliseconds: 50));
      Iterable<String> ids() =>
          subscription.read().requireValue.map((e) => e.id);
      settle();
      expect(ids(), hasLength(50));
      expect(ids(), isNot(contains(notice.id)));
      expect(_noticeReads(relay).single['since'], 50);

      final notifier = container.read(
        channelMessagesProvider(_channelId).notifier,
      );
      notifier.fetchOlder().then((loaded) => expect(loaded, isTrue));
      settle();
      expect(ids(), hasLength(101));
      expect(ids().where((id) => id == notice.id), hasLength(1));
      expect(_noticeReads(relay), hasLength(2));
      expect(_noticeReads(relay).last['since'], 50);
      notifier.fetchOlder().then((loaded) => expect(loaded, isTrue));
      settle();
      expect(ids(), hasLength(122));
      expect(ids(), contains(beforeMessages.id));
      expect(_noticeReads(relay).last['since'], isNull);
    });
  });

  for (final hasMessage in [false, true]) {
    test('channel start retains earlier notices: message=$hasMessage', () {
      fakeAsync((clock) {
        final relay = FakeRelay();
        final notice = _notice(relay, 0, 'First activity');
        relay.seed([if (hasMessage) _message(relay, 10), notice]);
        final container = ProviderContainer(overrides: relay.overrides);
        addTearDown(container.dispose);
        final subscription = container.listen(
          channelMessagesProvider(_channelId),
          (_, _) {},
        );
        addTearDown(subscription.close);
        clock.elapse(const Duration(milliseconds: 50));
        expect(
          subscription.read().requireValue.map((event) => event.id),
          contains(notice.id),
        );
        expect(_noticeReads(relay).single['since'], isNull);
      });
    });
  }

  testWidgets('scrolling back recovers notices after a failed read and '
      'interleaves them with older messages', (tester) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final relay = FakeRelay();
    final notice = _notice(relay, 75, 'Older topic');
    relay.seed([for (var i = 1; i <= 151; i++) _message(relay, i), notice]);
    relay.failQuery(
      (filter) => (filter['kinds'] as List).singleOrNull == 40099,
    );
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          ...relay.overrides,
          savedPrefsProvider.overrideWithValue(prefs),
          appLifecycleProvider.overrideWith(_Resumed.new),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: ChannelDetailPage(
            channel: Channel(
              id: _channelId,
              name: 'general',
              description: 'General discussion',
              channelType: 'stream',
              visibility: 'open',
              createdBy: relay.pubkey,
              createdAt: DateTime(2025),
              memberCount: 1,
              isMember: true,
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Message 151'), findsOneWidget);
    expect(find.textContaining('Older topic'), findsNothing);

    // Drive the actual timeline's older-page trigger with scroll gestures.
    for (
      var i = 0;
      i < 30 && find.textContaining('Older topic').evaluate().isEmpty;
      i++
    ) {
      await tester.drag(find.byType(Scrollable).first, const Offset(0, 650));
      await tester.pumpAndSettle();
    }
    expect(find.textContaining('Older topic'), findsOneWidget);
    final container = ProviderScope.containerOf(
      tester.element(find.byType(ChannelDetailPage)),
    );
    final events = container
        .read(channelMessagesProvider(_channelId))
        .requireValue;
    final position = events.indexWhere((event) => event.id == notice.id);
    expect(position, greaterThan(0));
    expect(events[position - 1].createdAt, lessThanOrEqualTo(75));
    expect(events[position + 1].createdAt, greaterThanOrEqualTo(75));
    expect(events.where((event) => event.id == notice.id), hasLength(1));
    expect(_noticeReads(relay).map((filter) => filter['since']), contains(52));
  });
}

class _Resumed extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}
