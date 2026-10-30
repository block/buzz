import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/thread_detail_page.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/semantics.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart' show Override;
import 'package:image_picker/image_picker.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../../helpers/fake_relay.dart';

const _channelId = '11111111-2222-4333-8444-555555555555';
const _shrug = '¯\\_(ツ)_/¯';

Channel _channel(String type) => Channel(
  id: _channelId,
  name: 'general',
  channelType: type,
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

Future<FakeRelay> _pump(
  WidgetTester tester,
  Channel channel, {
  Widget? home,
  List<Override> overrides = const [],
}) async {
  SharedPreferences.setMockInitialValues({});
  final prefs = await SharedPreferences.getInstance();
  tester.view.physicalSize = const Size(390, 844);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  final relay = FakeRelay();
  await tester.pumpWidget(
    ProviderScope(
      key: UniqueKey(),
      overrides: [
        ...relay.overrides,
        savedPrefsProvider.overrideWithValue(prefs),
        appLifecycleProvider.overrideWith(_Resumed.new),
        ...overrides,
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: home ?? ChannelDetailPage(channel: channel),
      ),
    ),
  );
  await tester.pumpAndSettle();
  return relay;
}

void main() {
  testWidgets('slash commands: suggest, run, pass through, fail, and stay off '
      'in forums', (tester) async {
    final semantics = tester.ensureSemantics();
    final relay = await _pump(
      tester,
      _channel('stream'),
      overrides: [
        mediaUploadServiceProvider.overrideWithValue(
          MediaUploadService(
            baseUrl: 'https://relay.example',
            nsec: null,
            pickGalleryImage: () async => null,
            pickGalleryVideo: () async => null,
            pickAttachmentFile: () async => XFile('/tmp/slash-notes.txt'),
          ),
        ),
      ],
    );

    String draft() =>
        tester.widget<TextField>(find.byType(TextField)).controller!.text;
    Iterable<NostrEvent> sent() =>
        relay.published.where((e) => e.kind == EventKind.streamMessage);
    Future<void> type(String text) async {
      if (find.byType(TextField).evaluate().isEmpty) {
        await tester.tap(find.text('Message #general'));
        await tester.pumpAndSettle();
      }
      await tester.enterText(find.byType(TextField), text);
      await tester.pump();
    }

    Future<void> send() async {
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pumpAndSettle();
    }

    final popover = find.byKey(const ValueKey('slash-suggestions-popover'));

    await type('/');
    expect(popover, findsOneWidget);
    final row = find.descendant(of: popover, matching: find.byType(ListTile));
    expect(row, findsOneWidget);
    expect(find.text('/shrug'), findsNothing, reason: 'name and hint are one');
    final label = tester.getSemantics(row).label;
    expect(label, contains('/shrug [text]'));
    expect(label, contains('Send your text followed by'));
    expect(
      find.bySemanticsLabel(RegExp('^/shrug')),
      findsOneWidget,
      reason: 'one accessible label per row',
    );
    final data = tester.getSemantics(row).getSemanticsData();
    expect(data.flagsCollection.isButton, isTrue);
    expect(data.hasAction(SemanticsAction.tap), isTrue);

    await type('/zz');
    expect(popover, findsNothing);
    await type('/SH');
    expect(popover, findsOneWidget);

    await tester.tap(row);
    await tester.pump();
    expect(draft(), '/shrug ');
    expect(popover, findsNothing);
    expect(sent(), isEmpty, reason: 'choosing a row never runs the command');

    await type('/shrug  so it goes');
    await send();
    expect(sent().single.content, 'so it goes $_shrug');
    expect(sent().single.getTagValue('h'), _channelId);
    expect(draft(), isEmpty);
    expect(
      find.textContaining('so it goes $_shrug', findRichText: true),
      findsOneWidget,
    );

    await type('/review hello');
    await send();
    expect(sent().last.content, '/review hello');
    expect(draft(), isEmpty);

    await type('@Eva /shrug');
    await send();
    expect(sent().last.content, '@Eva /shrug');

    relay.rejectNext('blocked: not allowed here');
    await type('/shrug oops');
    await send();
    expect(draft(), '/shrug oops', reason: 'a failed command keeps the draft');
    expect(find.byType(SnackBar), findsOneWidget);

    await tester.pump(const Duration(seconds: 6));
    await tester.pumpAndSettle();
    await type('/shrug');
    await tester.tap(find.byTooltip('Add attachment').hitTestable());
    await tester.pumpAndSettle();
    await tester.tap(find.text('Files'));
    await tester.pumpAndSettle();
    expect(find.text('slash-notes.txt'), findsOneWidget);
    final before = sent().length;
    await send();
    expect(
      find.text('Commands cannot be sent with attachments'),
      findsOneWidget,
    );
    expect(sent().length, before);
    expect(draft(), '/shrug');
    expect(find.text('slash-notes.txt'), findsOneWidget);

    semantics.dispose();
  });

  testWidgets('a forum composer has no slash commands', (tester) async {
    await _pump(tester, _channel('forum'));
    await tester.tap(find.byTooltip('New post'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Write your post…'));
    await tester.pumpAndSettle();

    await tester.enterText(find.byType(TextField), '/');
    await tester.pump();

    expect(
      find.byKey(const ValueKey('slash-suggestions-popover')),
      findsNothing,
    );
  });

  testWidgets('in a thread, /shrug posts a thread reply', (tester) async {
    const headId =
        'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
    final head = TimelineMessage(
      id: headId,
      pubkey: 'bbbb',
      createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 - 60,
      content: 'thread head',
    );
    final relay = await _pump(
      tester,
      _channel('stream'),
      home: ThreadDetailPage(
        threadHead: head,
        allMessages: [head],
        channelId: _channelId,
        currentPubkey: null,
        isMember: true,
        isArchived: false,
      ),
    );

    await tester.tap(find.text('Reply in thread\u2026'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), '/shrug');
    await tester.pump();
    await tester.tap(find.byIcon(BuzzIcons.arrowUp));
    await tester.pumpAndSettle();

    final reply = relay.published
        .where((e) => e.kind == EventKind.streamMessage)
        .single;
    expect(reply.content, _shrug);
    expect(reply.getTagValue('h'), _channelId);
    expect(reply.getTagValue('e'), headId);
    relay.unanswerable.removeWhere((r) => r.contains('thread_cursor'));
  });
}
