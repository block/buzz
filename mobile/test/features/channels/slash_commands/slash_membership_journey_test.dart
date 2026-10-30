import 'dart:convert';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/mentions/mention_candidates_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

import '../../../helpers/fake_relay.dart';

const _general = '11111111-2222-4333-8444-555555555555';
const _design = '22222222-2222-4333-8444-555555555555';
const _random = '33333333-2222-4333-8444-555555555555';

final _bob = nostr.Keys.generate();
final _carol = nostr.Keys.generate();

class _Resumed extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}

Channel _generalChannel(bool isMember) => Channel(
  id: _general,
  name: 'general',
  channelType: 'stream',
  visibility: 'open',
  description: 'General discussion',
  createdBy: 'abc123',
  createdAt: DateTime(2025),
  memberCount: 2,
  isMember: isMember,
);

void _seed(FakeRelay relay, {required String selfRole}) {
  NostrEvent meta(String id, String name) => relay.sign(
    kind: 39000,
    tags: [
      ['d', id],
      ['name', name],
      ['t', 'stream'],
      ['public'],
    ],
  );
  NostrEvent members(String id, List<List<String>> people) => relay.sign(
    kind: 39002,
    tags: [
      ['d', id],
      for (final p in people) ['p', ...p],
    ],
  );
  relay.seed([
    meta(_general, 'general'),
    meta(_design, 'design'),
    meta(_random, 'random'),
    members(_general, [
      [relay.pubkey, '', selfRole],
      [_bob.public, '', 'member'],
    ]),
    members(_design, [
      [_carol.public, '', 'owner'],
    ]),
    members(_random, [
      [relay.pubkey, '', 'member'],
    ]),
  ]);
}

Future<FakeRelay> _open(WidgetTester tester, {required String selfRole}) async {
  SharedPreferences.setMockInitialValues({});
  final prefs = await SharedPreferences.getInstance();
  tester.view.physicalSize = const Size(390, 844);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  final relay = FakeRelay();
  _seed(relay, selfRole: selfRole);
  await tester.pumpWidget(
    ProviderScope(
      key: UniqueKey(),
      overrides: [
        ...relay.overrides,
        savedPrefsProvider.overrideWithValue(prefs),
        appLifecycleProvider.overrideWith(_Resumed.new),
        mentionUserSearchProvider.overrideWith(
          (ref, query) async => query.trim().toLowerCase() == 'carol'
              ? [UserProfile(pubkey: _carol.public, displayName: 'Carol')]
              : const <UserProfile>[],
        ),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Builder(
          builder: (context) => Scaffold(
            body: TextButton(
              onPressed: () => Navigator.of(context).push(
                MaterialPageRoute<void>(
                  builder: (_) =>
                      ChannelDetailPage(channel: _generalChannel(true)),
                ),
              ),
              child: const Text('open general'),
            ),
          ),
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
  await tester.tap(find.text('open general'));
  await tester.pumpAndSettle();
  ProviderScope.containerOf(tester.element(find.byType(ChannelDetailPage)))
      .read(userCacheProvider.notifier)
      .cacheProfileEvent(
        relay.sign(
          kind: 0,
          tags: const [],
          author: _bob,
          content: jsonEncode({'display_name': 'Bob'}),
        ),
      );
  await tester.pumpAndSettle();
  return relay;
}

void main() {
  String draft(WidgetTester tester) =>
      tester.widget<TextField>(find.byType(TextField)).controller!.text;

  Future<void> type(WidgetTester tester, String text) async {
    if (find.byType(TextField).evaluate().isEmpty) {
      await tester.tap(find.text('Message #general'));
      await tester.pumpAndSettle();
    }
    await tester.enterText(find.byType(TextField), text);
    await tester.pump();
  }

  Future<void> mention(WidgetTester tester, String command, String name) async {
    await type(tester, '$command @$name');
    await tester.pump(const Duration(milliseconds: 400));
    await tester.pumpAndSettle();
    final row = find.descendant(
      of: find.byKey(const ValueKey('mention-suggestions-popover')),
      matching: find.textContaining(name),
    );
    await tester.tap(row.first);
    await tester.pumpAndSettle();
  }

  Future<void> send(WidgetTester tester) async {
    await tester.tap(find.byIcon(BuzzIcons.arrowUp));
    await tester.pumpAndSettle();
  }

  Future<void> expectToast(WidgetTester tester, String text) async {
    expect(find.textContaining(text), findsOneWidget);
    await tester.pump(const Duration(seconds: 6));
    await tester.pumpAndSettle();
  }

  testWidgets('owner runs the membership commands', (tester) async {
    final semantics = tester.ensureSemantics();
    final relay = await _open(tester, selfRole: 'owner');
    Iterable<NostrEvent> sent(int kind) =>
        relay.published.where((e) => e.kind == kind);

    for (final name in ['invite', 'join', 'leave', 'remove', 'who']) {
      await type(tester, '/${name.substring(0, 2)}');
      expect(find.bySemanticsLabel(RegExp('^/$name')), findsOneWidget);
    }
    for (final alias in ['ki', 'pa', 'mem']) {
      await type(tester, '/$alias');
      expect(
        find.byKey(const ValueKey('slash-suggestions-popover')),
        findsNothing,
        reason: 'aliases are never listed',
      );
    }

    await mention(tester, '/invite', 'Carol');
    await send(tester);
    final invite = sent(9000).single;
    expect(invite.getTagValue('h'), _general);
    expect(invite.getTagValue('p'), _carol.public);
    expect(draft(tester), isEmpty);
    await expectToast(tester, 'Added 1');

    await mention(tester, '/remove', 'Bob');
    await send(tester);
    expect(sent(9001).single.getTagValue('p'), _bob.public);
    expect(draft(tester), isEmpty);
    await expectToast(tester, 'Removed 1');

    await mention(tester, '/kick', 'Bob');
    await send(tester);
    expect(sent(9001), hasLength(2));
    await expectToast(tester, 'Removed 1');

    relay.rejectNext(
      'blocked: only owners can remove',
      match: (e) => e.kind == 9001,
    );
    await mention(tester, '/remove', 'Bob');
    final before = draft(tester);
    await send(tester);
    expect(draft(tester), before, reason: 'a rejected command keeps the draft');
    await expectToast(tester, 'only owners can remove');

    await type(tester, '/invite');
    await send(tester);
    expect(draft(tester), '/invite');
    await expectToast(tester, 'Usage: /invite @user…');

    await type(tester, '/join #nowhere');
    await send(tester);
    await expectToast(tester, 'No channel named #nowhere');

    relay.rejectNext(
      'blocked: channel is private',
      match: (e) => e.kind == 9021,
    );
    await type(tester, '/join #design');
    await send(tester);
    expect(draft(tester), '/join #design');
    expect(find.text('Message #design'), findsNothing);
    await expectToast(tester, 'channel is private');

    await send(tester);
    expect(sent(9021), hasLength(2));
    expect(find.text('design'), findsWidgets);
    await tester.pageBack();
    await tester.pumpAndSettle();
    expect(find.text('Message #general'), findsOneWidget);

    await type(tester, '/join #random');
    await send(tester);
    expect(sent(9021), hasLength(2), reason: 'a member only opens it');
    expect(find.text('Message #random'), findsOneWidget);
    await tester.pageBack();
    await tester.pumpAndSettle();
    expect(find.text('Message #general'), findsOneWidget);

    await type(tester, '/who');
    await send(tester);
    expect(find.text('Members'), findsOneWidget);
    await tester.tapAt(const Offset(195, 20));
    await tester.pumpAndSettle();
    expect(find.text('Members'), findsNothing);
    expect(find.text('Message #general'), findsOneWidget);

    await type(tester, '/part');
    await tester.tap(find.byIcon(BuzzIcons.arrowUp));
    await tester.pump(const Duration(milliseconds: 500));
    expect(find.text('Leave #general?'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(sent(9022), isEmpty);
    expect(find.text('open general'), findsNothing);

    relay.rejectNext(
      'blocked: the last owner cannot leave',
      match: (e) => e.kind == 9022,
    );
    await type(tester, '/leave');
    await tester.tap(find.byIcon(BuzzIcons.arrowUp));
    await tester.pump(const Duration(milliseconds: 500));
    await tester.tap(find.text('Leave'));
    await tester.pumpAndSettle();
    expect(find.text('/leave'), findsWidgets, reason: 'the draft stays');
    expect(find.text('open general'), findsNothing);
    await expectToast(tester, 'the last owner cannot leave');

    await tester.tap(find.byIcon(BuzzIcons.arrowUp).hitTestable());
    await tester.pump(const Duration(milliseconds: 500));
    await tester.tap(find.text('Leave'));
    await tester.pumpAndSettle();
    expect(sent(9022), hasLength(2));
    expect(sent(9022).last.getTagValue('h'), _general);
    expect(find.text('open general'), findsOneWidget);

    semantics.dispose();
  });

  testWidgets('a plain member is not offered /remove', (tester) async {
    await _open(tester, selfRole: 'member');

    await type(tester, '/in');
    expect(find.bySemanticsLabel(RegExp('^/invite')), findsOneWidget);
    await type(tester, '/re');
    expect(find.bySemanticsLabel(RegExp('^/remove')), findsNothing);
  });
}
