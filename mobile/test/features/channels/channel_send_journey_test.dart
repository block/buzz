import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/fake_relay.dart';

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

void main() {
  testWidgets('a sent message shows at once, settles, and a rejected one '
      'comes back to the composer', (tester) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final relay = FakeRelay();
    relay.seed([
      relay.sign(
        kind: EventKind.streamMessage,
        content: 'earlier message',
        createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 - 60,
        tags: [
          ['h', _channelId],
        ],
      ),
    ]);
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          ...relay.overrides,
          savedPrefsProvider.overrideWithValue(prefs),
          appLifecycleProvider.overrideWith(_Resumed.new),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: ChannelDetailPage(channel: _channel),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('earlier message'), findsOneWidget);

    Future<void> send(String text) async {
      if (find.byType(TextField).evaluate().isEmpty) {
        await tester.tap(find.text('Message #general'));
        await tester.pumpAndSettle();
      }
      await tester.enterText(find.byType(TextField), text);
      await tester.pump();
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pump();
    }

    String draft() =>
        tester.widget<TextField>(find.byType(TextField)).controller!.text;

    final ok = relay.holdOk();
    await send('hello team');
    expect(find.text('hello team'), findsOneWidget);
    expect(draft(), isEmpty);
    expect(find.bySemanticsLabel('Sending message'), findsOneWidget);
    final sent = relay.published.where(
      (e) => e.kind == EventKind.streamMessage,
    );
    expect(sent.single.content, 'hello team');
    expect(sent.single.getTagValue('h'), _channelId);

    ok.complete();
    await tester.pumpAndSettle();
    expect(find.text('hello team'), findsOneWidget);
    expect(find.bySemanticsLabel('Sending message'), findsNothing);
    expect(find.byType(SnackBar), findsNothing);

    relay.rejectNext('blocked: not allowed here');
    await send('nope');
    await tester.pumpAndSettle();
    expect(find.text('nope'), findsOneWidget, reason: 'only the draft');
    expect(draft(), 'nope');
    expect(find.byType(SnackBar), findsOneWidget);
    expect(find.text('hello team'), findsOneWidget);
  });
}

class _Resumed extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}
