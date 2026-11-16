import 'package:buzz/features/channels/media_viewer_page.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/adaptive_conversation_pane.dart';
import 'package:buzz/shared/widgets/conversation_layout.dart';
import 'package:buzz/shared/widgets/frosted_app_bar.dart';
import 'package:buzz/shared/widgets/frosted_scaffold.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _landscape = Size(1210, 834);
const _portrait = Size(834, 1210);

void main() {
  test('tablet layout docks, hides and places threads by available width', () {
    // listWidth matches the pane's default sidebar for each window width.
    for (final row in [
      // 11" landscape: docked; a thread sits beside and hides the sidebar.
      (
        w: 1210.0,
        list: 360.0,
        sel: true,
        thread: false,
        hid: false,
        ov: false,
        tw: null,
        hold: null,
        sidebar: ConversationSidebarMode.docked,
        mode: ConversationThreadMode.none,
        detail: 849.0,
      ),
      (
        w: 1210.0,
        list: 360.0,
        sel: true,
        thread: true,
        hid: false,
        ov: false,
        tw: null,
        hold: null,
        sidebar: ConversationSidebarMode.hidden,
        mode: ConversationThreadMode.beside,
        detail: 769.0,
      ),
      // 13" landscape keeps all three panes.
      (
        w: 1376.0,
        list: 360.0,
        sel: true,
        thread: true,
        hid: false,
        ov: false,
        tw: null,
        hold: null,
        sidebar: ConversationSidebarMode.docked,
        mode: ConversationThreadMode.beside,
        detail: 574.0,
      ),
      // The user's choice hides a sidebar that fits; the overlay shows it.
      (
        w: 1210.0,
        list: 360.0,
        sel: true,
        thread: false,
        hid: true,
        ov: false,
        tw: null,
        hold: null,
        sidebar: ConversationSidebarMode.hidden,
        mode: ConversationThreadMode.none,
        detail: 1210.0,
      ),
      (
        w: 1210.0,
        list: 360.0,
        sel: true,
        thread: false,
        hid: true,
        ov: true,
        tw: null,
        hold: null,
        sidebar: ConversationSidebarMode.overlay,
        mode: ConversationThreadMode.none,
        detail: 1210.0,
      ),
      // 11" portrait hides it while a conversation is open, and a thread
      // covers the conversation.
      (
        w: 834.0,
        list: 288.0,
        sel: true,
        thread: false,
        hid: false,
        ov: false,
        tw: null,
        hold: null,
        sidebar: ConversationSidebarMode.hidden,
        mode: ConversationThreadMode.none,
        detail: 834.0,
      ),
      (
        w: 834.0,
        list: 288.0,
        sel: true,
        thread: true,
        hid: false,
        ov: false,
        tw: null,
        hold: null,
        sidebar: ConversationSidebarMode.hidden,
        mode: ConversationThreadMode.covering,
        detail: 834.0,
      ),
      // With nothing selected the list always shows, even if hidden before.
      (
        w: 834.0,
        list: 288.0,
        sel: false,
        thread: true,
        hid: true,
        ov: true,
        tw: null,
        hold: null,
        sidebar: ConversationSidebarMode.docked,
        mode: ConversationThreadMode.none,
        detail: 545.0,
      ),
      // A dragged thread width is kept within what leaves the conversation
      // 480 pt, and a narrow one makes room for the sidebar again.
      (
        w: 1210.0,
        list: 360.0,
        sel: true,
        thread: true,
        hid: false,
        ov: false,
        tw: 600.0,
        hold: null,
        sidebar: ConversationSidebarMode.hidden,
        mode: ConversationThreadMode.beside,
        detail: 609.0,
      ),
      (
        w: 1210.0,
        list: 360.0,
        sel: true,
        thread: true,
        hid: false,
        ov: false,
        tw: 2000.0,
        hold: null,
        sidebar: ConversationSidebarMode.hidden,
        mode: ConversationThreadMode.beside,
        detail: 480.0,
      ),
      (
        w: 1210.0,
        list: 360.0,
        sel: true,
        thread: true,
        hid: false,
        ov: false,
        tw: 320.0,
        hold: null,
        sidebar: ConversationSidebarMode.docked,
        mode: ConversationThreadMode.beside,
        detail: 528.0,
      ),
      // Mid-drag the sidebar stays where it was: hidden here, and docked on
      // a 13" iPad, where docking also caps how wide the thread can grow.
      (
        w: 1210.0,
        list: 360.0,
        sel: true,
        thread: true,
        hid: false,
        ov: false,
        tw: 320.0,
        hold: false,
        sidebar: ConversationSidebarMode.hidden,
        mode: ConversationThreadMode.beside,
        detail: 889.0,
      ),
      (
        w: 1376.0,
        list: 360.0,
        sel: true,
        thread: true,
        hid: false,
        ov: false,
        tw: 900.0,
        hold: true,
        sidebar: ConversationSidebarMode.docked,
        mode: ConversationThreadMode.beside,
        detail: 480.0,
      ),
      // Portrait still covers, whatever width was dragged in landscape.
      (
        w: 834.0,
        list: 288.0,
        sel: true,
        thread: true,
        hid: false,
        ov: false,
        tw: 320.0,
        hold: null,
        sidebar: ConversationSidebarMode.hidden,
        mode: ConversationThreadMode.covering,
        detail: 834.0,
      ),
    ]) {
      final layout = ConversationLayout.resolve(
        width: row.w,
        listWidth: row.list,
        selected: row.sel,
        threadOpen: row.thread,
        userHidden: row.hid,
        overlayOpen: row.ov,
        threadWidth: row.tw,
        holdDocked: row.hold,
      );
      expect(layout.sidebar, row.sidebar, reason: '$row');
      expect(layout.thread, row.mode, reason: '$row');
      expect(layout.detailWidth, row.detail, reason: '$row');
      if (row.mode == ConversationThreadMode.beside) {
        expect(layout.threadLeft + layout.threadWidth, row.w, reason: '$row');
        expect(
          layout.detailLeft + layout.detailWidth + 1,
          layout.threadLeft,
          reason: '$row',
        );
        expect(
          layout.threadWidth,
          lessThanOrEqualTo(layout.threadMaxWidth),
          reason: '$row',
        );
      }
    }
  });

  testWidgets('tablet sidebar hides, overlays and makes room for a thread', (
    tester,
  ) async {
    final controller = ConversationPaneController();
    addTearDown(controller.dispose);
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = _landscape;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      ProviderScope(
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: AdaptiveConversationPane(
              controller: controller,
              list: Builder(
                builder: (context) => Column(
                  children: [
                    for (final name in ['First', 'Second'])
                      TextButton(
                        onPressed: () => openConversationPane(
                          context,
                          name,
                          (_) => _TabletConversation(name),
                        ),
                        child: Text('$name chat'),
                      ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
    final list = find.text('First chat');

    // Landscape docks the list beside the conversation; the header hides it.
    await tester.tap(list);
    await tester.pumpAndSettle();
    expect(list, findsOneWidget);
    await tester.enterText(
      find.byKey(const ValueKey('conversation-field')),
      'Keep my draft',
    );
    await tester.tap(find.byTooltip('Hide sidebar'));
    await tester.pumpAndSettle();
    expect(list, findsNothing);
    expect(
      tester.getTopLeft(find.byKey(const ValueKey('conversation-field'))).dx,
      0,
    );
    await tester.tap(find.byTooltip('Show sidebar'));
    await tester.pumpAndSettle();
    expect(list, findsOneWidget);

    // A thread opens beside the conversation and the sidebar makes room.
    await tester.tap(find.text('Reply in thread'));
    await tester.pumpAndSettle();
    expect(find.text('Thread content'), findsOneWidget);
    expect(list, findsNothing);
    expect(
      tester.getTopLeft(find.text('Thread content')).dx,
      greaterThan(tester.getTopRight(find.text('First conversation')).dx),
    );
    await tester.enterText(
      find.byKey(const ValueKey('thread-field')),
      'Thread reply draft',
    );

    // The divider drags to resize the thread; the sidebar stays put mid-drag
    // and returns once a narrow thread leaves it room.
    final threadPane = find.byKey(const ValueKey('conversation-thread'));
    expect(tester.getSize(threadPane).width, 440);
    final drag = await tester.startGesture(
      tester.getCenter(
        find.byKey(const ValueKey('conversation-thread-divider')),
      ),
    );
    await drag.moveBy(const Offset(-30, 0));
    await tester.pump();
    final started = tester.getSize(threadPane).width;
    await drag.moveBy(const Offset(-100, 0));
    await tester.pump();
    expect(tester.getSize(threadPane).width, started + 100);
    expect(tester.getTopLeft(threadPane).dx, _landscape.width - started - 100);
    await drag.moveBy(const Offset(400, 0));
    await tester.pump();
    expect(tester.getSize(threadPane).width, 320);
    expect(list, findsNothing);
    await drag.up();
    await tester.pumpAndSettle();
    expect(list, findsOneWidget);
    expect(find.text('Thread reply draft'), findsOneWidget);

    // Screen readers resize it as a slider.
    final handle = find.bySemanticsLabel('Thread width');
    expect(tester.getSemantics(handle).value, '320 points');
    tester.semantics.increase(find.semantics.byLabel('Thread width'));
    await tester.pumpAndSettle();
    expect(tester.getSize(threadPane).width, 360);

    // Portrait has no room beside: the same thread covers the conversation,
    // keeping its draft, and Back returns to the conversation.
    tester.view.physicalSize = _portrait;
    await tester.pumpAndSettle();
    expect(find.text('Thread reply draft'), findsOneWidget);
    expect(
      tester.getTopLeft(find.text('Thread content')).dx,
      lessThan(tester.getTopRight(find.text('First conversation')).dx),
    );
    await tester.tap(find.byTooltip('Back'));
    await tester.pumpAndSettle();
    expect(find.text('Thread content'), findsNothing);
    expect(find.text('Keep my draft'), findsOneWidget);

    // Portrait hides the list until asked; picking a chat dismisses it.
    expect(list, findsNothing);
    await tester.tap(find.byTooltip('Show sidebar'));
    await tester.pumpAndSettle();
    expect(list, findsOneWidget);
    expect(find.bySemanticsLabel('Close sidebar'), findsOneWidget);
    await tester.tap(find.text('Second chat'));
    await tester.pumpAndSettle();
    expect(find.text('Second conversation'), findsOneWidget);
    expect(list, findsNothing);

    // Keyboard: Control-Command-S reveals the sidebar and Escape dismisses it,
    // moving focus into the sidebar and back to the composer.
    bool composerFocused() => tester
        .widget<EditableText>(
          find.descendant(
            of: find.byKey(const ValueKey('conversation-field')),
            matching: find.byType(EditableText),
          ),
        )
        .focusNode
        .hasFocus;
    await tester.showKeyboard(find.byKey(const ValueKey('conversation-field')));
    await tester.pump();
    expect(composerFocused(), isTrue);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyS);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pumpAndSettle();
    expect(list, findsOneWidget);
    expect(composerFocused(), isFalse);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(list, findsNothing);
    expect(composerFocused(), isTrue);

    // The overlay is modal: screen readers skip the conversation behind it,
    // and system Back dismisses it before it would close the conversation.
    await tester.tap(find.byTooltip('Show sidebar'));
    await tester.pumpAndSettle();
    expect(list, findsOneWidget);
    expect(find.semantics.byLabel('Hide sidebar'), findsNothing);
    expect(find.semantics.byLabel('Second conversation'), findsNothing);
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(list, findsNothing);
    expect(find.text('Second conversation'), findsOneWidget);
    expect(find.semantics.byLabel('Second conversation'), findsOne);

    // A second Back while the overlay slides away is spent on the overlay,
    // so a double press doesn't close the conversation.
    await tester.tap(find.byTooltip('Show sidebar'));
    await tester.pumpAndSettle();
    await tester.binding.handlePopRoute();
    await tester.pump(const Duration(milliseconds: 60));
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(list, findsNothing);
    expect(find.text('Second conversation'), findsOneWidget);

    // Rotating an open overlay into a docked sidebar spends it: Hide then
    // hides, and rotating back does not reopen it.
    await tester.tap(find.byTooltip('Show sidebar'));
    await tester.pumpAndSettle();
    tester.view.physicalSize = _landscape;
    await tester.pumpAndSettle();
    expect(list, findsOneWidget);
    await tester.tap(find.byTooltip('Hide sidebar'));
    await tester.pumpAndSettle();
    expect(list, findsNothing);
    expect(find.bySemanticsLabel('Close sidebar'), findsNothing);
    tester.view.physicalSize = _portrait;
    await tester.pumpAndSettle();
    expect(list, findsNothing);

    // So does a window that turns compact, as in Split View.
    await tester.tap(find.byTooltip('Show sidebar'));
    await tester.pumpAndSettle();
    expect(list, findsOneWidget);
    tester.view.physicalSize = const Size(500, 900);
    await tester.pumpAndSettle();
    tester.view.physicalSize = _portrait;
    await tester.pumpAndSettle();
    expect(list, findsNothing);

    // System Back closes an open thread, not the conversation under it.
    await tester.tap(find.text('Reply in thread'));
    await tester.pumpAndSettle();
    expect(find.text('Thread content'), findsOneWidget);
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(find.text('Thread content'), findsNothing);
    expect(find.text('Second conversation'), findsOneWidget);

    // A thread belongs to its conversation page: a page pushed above it
    // closes the pane, and threads opened from that page push over it.
    tester.view.physicalSize = _landscape;
    await tester.pumpAndSettle();
    await tester.tap(find.text('Reply in thread'));
    await tester.pumpAndSettle();
    expect(find.text('Thread content'), findsOneWidget);
    // The media viewer only shows over the conversation: the thread stays,
    // and Back closes the viewer first.
    await tester.tap(find.text('Open image'));
    await tester.pumpAndSettle();
    expect(find.byType(MediaImageViewerPage), findsOneWidget);
    expect(find.text('Thread content'), findsOneWidget);
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(find.byType(MediaImageViewerPage), findsNothing);
    expect(find.text('Thread content'), findsOneWidget);
    await tester.tap(find.text('Open linked page'));
    await tester.pumpAndSettle();
    expect(find.text('Linked page'), findsOneWidget);
    expect(find.text('Thread content'), findsNothing);
    await tester.tap(find.text('Reply in linked thread'));
    await tester.pumpAndSettle();
    expect(find.text('Linked thread content'), findsOneWidget);
    expect(find.byKey(const ValueKey('conversation-thread')), findsNothing);
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(find.text('Linked page'), findsNothing);
    expect(find.text('Second conversation'), findsOneWidget);
    expect(find.text('Thread content'), findsNothing);
    expect(tester.takeException(), isNull);
  });
}

class _TabletConversation extends StatelessWidget {
  const _TabletConversation(this.name);

  final String name;

  @override
  Widget build(BuildContext context) => FrostedScaffold(
    appBar: FrostedAppBar(
      automaticallyImplyLeading: !ConversationPaneScope.isBaseRouteOf(context),
      title: Text('$name conversation'),
    ),
    body: Column(
      children: [
        const TextField(key: ValueKey('conversation-field')),
        TextButton(
          onPressed: () => openThreadPane(
            context,
            (_) => FrostedScaffold(
              appBar: const FrostedAppBar(title: Text('Thread')),
              body: const Column(
                children: [
                  Text('Thread content'),
                  TextField(key: ValueKey('thread-field')),
                ],
              ),
            ),
          ),
          child: const Text('Reply in thread'),
        ),
        TextButton(
          onPressed: () => Navigator.of(
            context,
          ).push(MaterialPageRoute<void>(builder: (_) => const _LinkedPage())),
          child: const Text('Open linked page'),
        ),
        TextButton(
          onPressed: () => openImageViewer(
            context,
            imageUrl: 'https://example.org/photo.png',
            heroTag: 'tablet-photo',
          ),
          child: const Text('Open image'),
        ),
      ],
    ),
  );
}

class _LinkedPage extends StatelessWidget {
  const _LinkedPage();

  @override
  Widget build(BuildContext context) => FrostedScaffold(
    appBar: const FrostedAppBar(title: Text('Linked page')),
    body: Center(
      child: TextButton(
        onPressed: () => openThreadPane(
          context,
          (_) => const Scaffold(body: Text('Linked thread content')),
        ),
        child: const Text('Reply in linked thread'),
      ),
    ),
  );
}
