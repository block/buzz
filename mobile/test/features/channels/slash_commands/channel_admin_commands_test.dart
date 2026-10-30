import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../../helpers/fake_relay.dart';

const _channelId = '11111111-2222-4333-8444-555555555555';

final _channel = Channel(
  id: _channelId,
  name: 'general',
  channelType: 'stream',
  visibility: 'open',
  description: 'General discussion',
  createdBy: 'abc123',
  createdAt: DateTime(2025),
  memberCount: 5,
  isMember: true,
);

class _Resumed extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}

Future<FakeRelay> _pump(WidgetTester tester, {required String myRole}) async {
  SharedPreferences.setMockInitialValues({});
  final prefs = await SharedPreferences.getInstance();
  tester.view.physicalSize = const Size(390, 844);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  final relay = FakeRelay();
  relay.seed([
    relay.sign(
      kind: 39002,
      tags: [
        ['d', _channelId],
        ['p', relay.pubkey, '', myRole],
      ],
    ),
    relay.sign(
      kind: 39000,
      tags: [
        ['d', _channelId],
        ['name', 'general'],
        ['t', 'stream'],
        ['public'],
      ],
    ),
  ]);
  await tester.pumpWidget(
    ProviderScope(
      key: UniqueKey(),
      overrides: [
        ...relay.overrides,
        savedPrefsProvider.overrideWithValue(prefs),
        appLifecycleProvider.overrideWith(_Resumed.new),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Builder(
          builder: (context) => Scaffold(
            body: TextButton(
              onPressed: () => Navigator.of(context).push(
                MaterialPageRoute<void>(
                  builder: (_) => ChannelDetailPage(channel: _channel),
                ),
              ),
              child: const Text('Open channel'),
            ),
          ),
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
  await tester.tap(find.text('Open channel'));
  await tester.pumpAndSettle();
  return relay;
}

void main() {
  testWidgets('channel admin commands: rename, mute, archive, rejection, and '
      'visibility for a plain member', (tester) async {
    final semantics = tester.ensureSemantics();
    final relay = await _pump(tester, myRole: 'admin');

    Future<void> expand() async {
      if (find.byType(TextField).evaluate().isEmpty) {
        await tester.tap(find.text('Message #general'));
        await tester.pumpAndSettle();
      }
    }

    String draft() =>
        tester.widget<TextField>(find.byType(TextField)).controller!.text;
    Iterable<NostrEvent> updates() =>
        relay.published.where((e) => e.kind == 9002);
    Future<void> type(String text) async {
      await expand();
      await tester.enterText(find.byType(TextField), text);
      await tester.pump();
    }

    Future<void> send({bool settle = true}) async {
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      if (settle) {
        await tester.pumpAndSettle();
      } else {
        await tester.pump(const Duration(milliseconds: 400));
      }
    }

    Future<void> expectToast(String text) async {
      expect(find.text(text), findsOneWidget);
      await tester.pump(const Duration(seconds: 6));
      await tester.pumpAndSettle();
    }

    await type('/');
    for (final name in ['rename', 'archive', 'mute']) {
      expect(
        find.bySemanticsLabel(RegExp('^/$name')),
        findsOneWidget,
        reason: '/$name is suggested to an admin',
      );
    }

    await type('/mute');
    await send();
    await expectToast('Muted #general');
    expect(draft(), isEmpty);
    await type('/mute');
    await send();
    await expectToast('Unmuted #general');

    await type('/rename #new-name');
    await send();
    expect(updates().single.getTagValue('h'), _channelId);
    expect(updates().single.getTagValue('name'), 'new-name');
    await expectToast('Renamed to #new-name');
    expect(draft(), isEmpty);

    relay.rejectNext('blocked: not allowed', match: (e) => e.kind == 9002);
    await type('/rename other');
    await send();
    expect(draft(), '/rename other');
    expect(find.textContaining('blocked: not allowed'), findsOneWidget);
    await tester.pump(const Duration(seconds: 6));
    await tester.pumpAndSettle();

    await type('/rename');
    await send();
    expect(find.textContaining('Usage: /rename <name>'), findsOneWidget);
    expect(draft(), '/rename');
    await tester.pump(const Duration(seconds: 6));
    await tester.pumpAndSettle();

    final before = updates().length;
    await type('/archive');
    await send(settle: false);
    expect(find.text('Archive #general?'), findsOneWidget);
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(find.byType(SnackBar), findsNothing, reason: 'cancel is silent');
    expect(updates().length, before, reason: 'cancel publishes nothing');
    expect(find.text('/archive'), findsWidgets, reason: 'draft is kept');
    await tester.pump(const Duration(seconds: 6));
    await tester.pumpAndSettle();

    await tester.tap(find.text('/archive').first);
    await tester.pumpAndSettle();
    expect(draft(), '/archive');
    await send(settle: false);
    await tester.tap(find.widgetWithText(FilledButton, 'Archive'));
    await tester.pumpAndSettle();
    expect(updates().length, before + 1);
    expect(updates().last.getTagValue('archived'), 'true');
    expect(find.text('Open channel'), findsOneWidget, reason: 'page closes');

    semantics.dispose();
  });

  testWidgets('a plain member is not offered /rename or /archive', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    await _pump(tester, myRole: 'member');

    await tester.tap(find.text('Message #general'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), '/');
    await tester.pump();

    expect(find.bySemanticsLabel(RegExp('^/mute')), findsOneWidget);
    expect(find.bySemanticsLabel(RegExp('^/rename')), findsNothing);
    expect(find.bySemanticsLabel(RegExp('^/archive')), findsNothing);
    semantics.dispose();
  });
}
