import 'dart:ui' as ui;

import 'package:buzz/features/channels/message_actions.dart';
import 'package:buzz/features/channels/message_long_press_region.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/features/channels/thread_follows/thread_follows_provider.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/reminders/reminder_service.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

const _channelId = 'chan-1';

TimelineMessage _message({
  String id = 'msg-1',
  String pubkey = 'alice',
  int createdAt = 1000,
  bool isSystem = false,
  String? rootId,
}) => TimelineMessage(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  content: 'hello world',
  isSystem: isSystem,
  rootId: rootId,
);

class _FakeReadStateNotifier extends ReadStateNotifier {
  final ReadStateState _initialState;
  final Map<String, int> markedRead = {};
  final List<String> markedUnread = [];

  _FakeReadStateNotifier(this._initialState);

  @override
  ReadStateState build() => _initialState;

  @override
  void markContextRead(
    String contextId,
    int unixTimestamp, {
    bool clearForcedMessages = false,
  }) {
    markedRead[contextId] = unixTimestamp;
    var forced = {
      for (final entry in state.forcedUnreadContexts.entries)
        if (entry.key != contextId) entry.key: entry.value,
    };
    if (clearForcedMessages) {
      forced = {
        for (final entry in forced.entries)
          if (entry.value != contextId) entry.key: entry.value,
      };
    }
    state = _withForced(forced).copyWithContext(contextId, unixTimestamp);
  }

  @override
  void markContextUnread(String contextId, {required String channelId}) {
    markedUnread.add(contextId);
    state = _withForced({...state.forcedUnreadContexts, contextId: channelId});
  }

  ReadStateState _withForced(Map<String, String> forced) => ReadStateState(
    isReady: state.isReady,
    pubkey: state.pubkey,
    contexts: state.contexts,
    version: state.version + 1,
    forcedUnreadContexts: Map.unmodifiable(forced),
  );
}

ReadStateState _readState(Map<String, int> contexts, {bool isReady = true}) =>
    ReadStateState(
      isReady: isReady,
      pubkey: 'self',
      contexts: contexts,
      version: 1,
    );

Future<SharedPreferences> _mockPrefs() async {
  SharedPreferences.setMockInitialValues({});
  return SharedPreferences.getInstance();
}

/// A [ReminderService] whose constructor dependencies are inert until used —
/// enough for visibility checks on the "Remind me" fast action.
ReminderService _stubReminderService() {
  final keys = nostr.Keys.generate();
  return ReminderService(
    signedEventRelay: SignedEventRelay(
      session: RelaySessionNotifier(),
      nsec: keys.nsec,
    ),
    crypto: ReminderCrypto(keys.nsec, keys.public),
  );
}

Future<void> _pumpSheet(
  WidgetTester tester, {
  required TimelineMessage message,
  required SharedPreferences prefs,
  ReadStateNotifier Function()? readStateOverride,
  bool canManageMessage = false,
  List<TimelineMessage>? allMessages,
  ReminderService? reminderService,
}) async {
  await tester.pumpWidget(
    ProviderScope(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        myPubkeyProvider.overrideWithValue('self'),
        readStateProvider.overrideWith(
          readStateOverride ??
              () => _FakeReadStateNotifier(
                _readState(const {_channelId: 100000}),
              ),
        ),
        // No signing identity → "Remind me" hidden by default; individual
        // tests opt in by passing a stub service.
        reminderServiceProvider.overrideWithValue(reminderService),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: Consumer(
            builder: (context, ref, _) => TextButton(
              onPressed: () => showMessageActions(
                context: context,
                ref: ref,
                message: message,
                channelId: _channelId,
                canManageMessage: canManageMessage,
                allMessages: allMessages,
                currentPubkey: 'self',
                isMember: true,
              ),
              child: const Text('open'),
            ),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('open'));
  await tester.pumpAndSettle();
}

Future<void> _pumpImageSheet(
  WidgetTester tester, {
  required TimelineMessage message,
  bool canManageMessage = false,
}) async {
  await tester.pumpWidget(
    ProviderScope(
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: Consumer(
            builder: (context, ref, _) => TextButton(
              onPressed: () => showImageActions(
                context: context,
                ref: ref,
                message: message,
                channelId: _channelId,
                imageUrl: 'https://example.com/photo.png',
                canManageMessage: canManageMessage,
              ),
              child: const Text('open image actions'),
            ),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('open image actions'));
  await tester.pumpAndSettle();
}

Future<ui.Image> _testMessageSnapshot() async {
  final recorder = ui.PictureRecorder();
  final canvas = ui.Canvas(recorder);
  canvas.drawRect(
    const Rect.fromLTWH(0, 0, 300, 72),
    ui.Paint()..color = const Color(0xffeeeeee),
  );
  return recorder.endRecording().toImage(300, 72);
}

class _MessageActionsPopoverHarness {
  final ProviderContainer container;
  final ValueNotifier<bool> sourceHidden;

  const _MessageActionsPopoverHarness({
    required this.container,
    required this.sourceHidden,
  });
}

Future<_MessageActionsPopoverHarness> _pumpMessageActionsPopover(
  WidgetTester tester, {
  required TimelineMessage message,
  required SharedPreferences prefs,
  ReadStateNotifier Function()? readStateOverride,
  bool canManageMessage = false,
  List<TimelineMessage>? allMessages,
  ReminderService? reminderService,
  bool disableAnimations = false,
  EdgeInsets viewInsets = EdgeInsets.zero,
  FocusNode? composerFocusNode,
  bool composerInitiallyFocused = false,
}) async {
  final sourceHidden = ValueNotifier(false);

  await tester.pumpWidget(
    ProviderScope(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        myPubkeyProvider.overrideWithValue('self'),
        readStateProvider.overrideWith(
          readStateOverride ??
              () => _FakeReadStateNotifier(
                _readState(const {_channelId: 100000}),
              ),
        ),
        reminderServiceProvider.overrideWithValue(reminderService),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(context).copyWith(
            disableAnimations: disableAnimations,
            viewInsets: viewInsets,
          ),
          child: child!,
        ),
        home: Scaffold(
          body: Consumer(
            builder: (context, ref, _) => Column(
              children: [
                if (composerFocusNode != null)
                  TextField(focusNode: composerFocusNode),
                TextButton(
                  key: const ValueKey('open-message-actions-popover'),
                  onPressed: () => showMessageActions(
                    context: context,
                    ref: ref,
                    message: message,
                    channelId: _channelId,
                    canManageMessage: canManageMessage,
                    allMessages: allMessages,
                    currentPubkey: 'self',
                    isMember: true,
                    anchorRect: const Rect.fromLTWH(32, 260, 300, 72),
                    captureAnchorSnapshot: _testMessageSnapshot,
                    onPopoverPresented: () => sourceHidden.value = true,
                    onPopoverDismissed: () => sourceHidden.value = false,
                    composerFocusNode: composerFocusNode,
                    restoreComposerFocus: composerFocusNode?.requestFocus,
                  ),
                  child: const Text('open message actions'),
                ),
              ],
            ),
          ),
        ),
      ),
    ),
  );
  if (composerInitiallyFocused) {
    composerFocusNode!.requestFocus();
    await tester.pump();
  }
  await tester.tap(find.byKey(const ValueKey('open-message-actions-popover')));
  await tester.pumpAndSettle();
  final container = ProviderScope.containerOf(
    tester.element(find.byKey(const ValueKey('open-message-actions-popover'))),
  );
  return _MessageActionsPopoverHarness(
    container: container,
    sourceHidden: sourceHidden,
  );
}

Future<void> _dismissMessageActionsPopover(WidgetTester tester) async {
  Navigator.of(
    tester.element(find.byKey(const ValueKey('message-action-surface'))),
  ).pop();
  await tester.pumpAndSettle();
}

void main() {
  testWidgets(
    'message long press keeps taps and scrolling while repeated holds win',
    (tester) async {
      var parentTaps = 0;
      var nestedTaps = 0;
      var longPresses = 0;
      final scrollController = ScrollController();
      addTearDown(scrollController.dispose);

      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: SingleChildScrollView(
              controller: scrollController,
              child: Column(
                children: [
                  Material(
                    child: MessageLongPressInkWell(
                      key: const ValueKey('parent-gesture-target'),
                      onTap: () => parentTaps += 1,
                      onLongPress: (_) => longPresses += 1,
                      child: const SizedBox(height: 80, width: 300),
                    ),
                  ),
                  Material(
                    child: MessageLongPressInkWell(
                      onLongPress: (_) => longPresses += 1,
                      child: GestureDetector(
                        key: const ValueKey('nested-gesture-target'),
                        behavior: HitTestBehavior.opaque,
                        onTap: () => nestedTaps += 1,
                        child: const SizedBox(height: 80, width: 300),
                      ),
                    ),
                  ),
                  const SizedBox(height: 900),
                ],
              ),
            ),
          ),
        ),
      );

      await tester.tap(find.byKey(const ValueKey('parent-gesture-target')));
      await tester.tap(find.byKey(const ValueKey('nested-gesture-target')));
      await tester.pump();
      expect(parentTaps, 1);
      expect(nestedTaps, 1);

      for (var index = 0; index < 5; index++) {
        await tester.longPress(
          find.byKey(const ValueKey('nested-gesture-target')),
        );
        await tester.pump();
        expect(longPresses, index + 1);
        expect(nestedTaps, 1);
      }

      final drag = await tester.startGesture(
        tester.getCenter(find.byKey(const ValueKey('parent-gesture-target'))),
      );
      await drag.moveBy(const Offset(0, -30));
      await tester.pump();
      await drag.moveBy(const Offset(0, -70));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 600));
      await drag.up();
      await tester.pumpAndSettle();

      expect(longPresses, 5);
      expect(scrollController.offset, greaterThan(0));
    },
  );

  testWidgets('iOS message long press recognizes at 200 ms', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    try {
      var longPresses = 0;

      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: Material(
              child: MessageLongPressInkWell(
                key: const ValueKey('ios-long-press-target'),
                onLongPress: (_) => longPresses += 1,
                child: const SizedBox(width: 240, height: 80),
              ),
            ),
          ),
        ),
      );

      final gesture = await tester.startGesture(
        tester.getCenter(find.byKey(const ValueKey('ios-long-press-target'))),
      );
      await tester.pump(const Duration(milliseconds: 199));
      expect(longPresses, 0);
      await tester.pump(const Duration(milliseconds: 2));
      expect(longPresses, 1);
      await gesture.up();
    } finally {
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets(
    'message long press captures the content inside the ink surface',
    (tester) async {
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetDevicePixelRatio);
      MessageLongPressDetails? longPressDetails;
      ui.Image? snapshot;
      addTearDown(() => snapshot?.dispose());

      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: Material(
              child: MessageLongPressInkWell(
                key: const ValueKey('snapshot-gesture-target'),
                onLongPressDetails: (details) => longPressDetails = details,
                child: const SizedBox(width: 240, height: 80),
              ),
            ),
          ),
        ),
      );

      await tester.longPress(
        find.byKey(const ValueKey('snapshot-gesture-target')),
      );
      expect(longPressDetails, isNotNull);

      final capture = longPressDetails!.captureSnapshot();
      await tester.pump();
      snapshot = await capture;

      expect(snapshot.width, 240);
      expect(snapshot.height, 80);
    },
  );

  testWidgets('message snapshot can exclude attached reaction content', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetDevicePixelRatio);
    final snapshotKey = GlobalKey();
    MessageLongPressDetails? longPressDetails;
    ui.Image? snapshot;
    addTearDown(() => snapshot?.dispose());

    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Material(
            child: MessageLongPressInkWell(
              key: const ValueKey('separate-snapshot-gesture-target'),
              snapshotKey: snapshotKey,
              onLongPressDetails: (details) => longPressDetails = details,
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  RepaintBoundary(
                    key: snapshotKey,
                    child: const SizedBox(width: 240, height: 80),
                  ),
                  Listener(
                    key: const ValueKey('attached-reactions'),
                    behavior: HitTestBehavior.opaque,
                    child: const SizedBox(width: 240, height: 32),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );

    await tester.longPress(find.byKey(const ValueKey('attached-reactions')));
    expect(longPressDetails, isNotNull);
    expect(longPressDetails!.anchorRect.height, 80);

    final capture = longPressDetails!.captureSnapshot();
    await tester.pump();
    snapshot = await capture;

    expect(snapshot.width, 240);
    expect(snapshot.height, 80);
  });

  group('showMessageActions', () {
    testWidgets('composes the tray, lifted preview, and compact actions', (
      tester,
    ) async {
      final prefs = await _mockPrefs();
      final harness = await _pumpMessageActionsPopover(
        tester,
        message: _message(),
        prefs: prefs,
        allMessages: [_message()],
        reminderService: _stubReminderService(),
      );

      expect(harness.sourceHidden.value, isTrue);
      expect(
        find.byKey(const ValueKey('message-action-reaction-tray')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('message-action-preview')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('message-action-surface')),
        findsOneWidget,
      );
      expect(find.byType(BottomSheet), findsNothing);
      expect(find.text('Reply'), findsOneWidget);
      expect(find.text('Copy link'), findsOneWidget);
      expect(find.text('Remind me'), findsOneWidget);
      expect(find.text('Follow thread'), findsOneWidget);

      final trayRect = tester.getRect(
        find.byKey(const ValueKey('message-action-reaction-tray')),
      );
      final previewRect = tester.getRect(
        find.byKey(const ValueKey('message-action-preview')),
      );
      final actionRect = tester.getRect(
        find.byKey(const ValueKey('message-action-surface')),
      );
      final trayMaterial = tester.widget<Material>(
        find.byKey(const ValueKey('message-action-reaction-tray')),
      );
      final actionMaterial = tester.widget<Material>(
        find.byKey(const ValueKey('message-action-surface')),
      );
      expect(previewRect.top, greaterThan(trayRect.bottom));
      expect(actionRect.top, greaterThan(previewRect.bottom));
      expect(previewRect.left, trayRect.left);
      expect(actionRect.left, trayRect.left);
      expect(actionRect.width, 288);
      expect(actionMaterial.color, trayMaterial.color);

      await _dismissMessageActionsPopover(tester);
      expect(harness.sourceHidden.value, isFalse);
    });

    for (final platform in [TargetPlatform.iOS, TargetPlatform.android]) {
      testWidgets(
        '${platform.name} composition keeps the action menu near the safe bottom',
        (tester) async {
          debugDefaultTargetPlatformOverride = platform;
          try {
            final prefs = await _mockPrefs();
            await _pumpMessageActionsPopover(
              tester,
              message: _message(),
              prefs: prefs,
              allMessages: [_message()],
              reminderService: _stubReminderService(),
            );

            final actionRect = tester.getRect(
              find.byKey(const ValueKey('message-action-surface')),
            );
            final logicalHeight =
                tester.view.physicalSize.height / tester.view.devicePixelRatio;
            expect(actionRect.bottom, closeTo(logicalHeight - Grid.xxs, 0.1));

            await _dismissMessageActionsPopover(tester);
          } finally {
            debugDefaultTargetPlatformOverride = null;
          }
        },
      );
    }

    testWidgets('keeps the action menu above the software keyboard', (
      tester,
    ) async {
      const keyboardInset = 300.0;
      final prefs = await _mockPrefs();
      await _pumpMessageActionsPopover(
        tester,
        message: _message(),
        prefs: prefs,
        allMessages: [_message()],
        reminderService: _stubReminderService(),
        viewInsets: const EdgeInsets.only(bottom: keyboardInset),
      );

      final actionRect = tester.getRect(
        find.byKey(const ValueKey('message-action-surface')),
      );
      final logicalHeight =
          tester.view.physicalSize.height / tester.view.devicePixelRatio;
      expect(
        actionRect.bottom,
        closeTo(logicalHeight - keyboardInset - Grid.xxs, 0.1),
      );

      await _dismissMessageActionsPopover(tester);
    });

    testWidgets('restores composer focus only after a dismissed popover', (
      tester,
    ) async {
      final focusNode = FocusNode();
      addTearDown(focusNode.dispose);
      final prefs = await _mockPrefs();

      await _pumpMessageActionsPopover(
        tester,
        message: _message(),
        prefs: prefs,
        composerFocusNode: focusNode,
        composerInitiallyFocused: true,
      );

      expect(focusNode.hasFocus, isFalse);
      await _dismissMessageActionsPopover(tester);
      expect(focusNode.hasFocus, isTrue);
    });

    testWidgets('leaves an initially unfocused composer unfocused', (
      tester,
    ) async {
      final focusNode = FocusNode();
      addTearDown(focusNode.dispose);
      final prefs = await _mockPrefs();

      await _pumpMessageActionsPopover(
        tester,
        message: _message(),
        prefs: prefs,
        composerFocusNode: focusNode,
      );

      expect(focusNode.hasFocus, isFalse);
      await _dismissMessageActionsPopover(tester);
      expect(focusNode.hasFocus, isFalse);
    });

    testWidgets('does not restore composer focus after selecting an action', (
      tester,
    ) async {
      final focusNode = FocusNode();
      addTearDown(focusNode.dispose);
      final prefs = await _mockPrefs();

      await _pumpMessageActionsPopover(
        tester,
        message: _message(rootId: 'root-9'),
        prefs: prefs,
        composerFocusNode: focusNode,
        composerInitiallyFocused: true,
      );

      await tester.tap(
        find.byKey(const ValueKey('message-action-followThread')),
      );
      await tester.pumpAndSettle();
      expect(focusNode.hasFocus, isFalse);
    });

    testWidgets('runs an action after dismissal and can reopen', (
      tester,
    ) async {
      final prefs = await _mockPrefs();
      final harness = await _pumpMessageActionsPopover(
        tester,
        message: _message(rootId: 'root-9'),
        prefs: prefs,
      );

      await tester.tap(
        find.byKey(const ValueKey('message-action-followThread')),
      );
      await tester.pumpAndSettle();

      expect(harness.sourceHidden.value, isFalse);
      expect(harness.container.read(threadFollowsProvider).followedRootIds, {
        'root-9',
      });

      await tester.tap(
        find.byKey(const ValueKey('open-message-actions-popover')),
      );
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('message-action-surface')),
        findsOneWidget,
      );
      await _dismissMessageActionsPopover(tester);
    });

    testWidgets('orders primary, utility, and destructive action groups', (
      tester,
    ) async {
      final prefs = await _mockPrefs();
      await _pumpMessageActionsPopover(
        tester,
        message: _message(),
        prefs: prefs,
        canManageMessage: true,
        allMessages: [_message()],
        reminderService: _stubReminderService(),
      );

      const actionIds = [
        'reply',
        'markUnread',
        'edit',
        'copyText',
        'copyLink',
        'remind',
        'followThread',
        'delete',
      ];
      final actionTops = [
        for (final actionId in actionIds)
          tester
              .getTopLeft(find.byKey(ValueKey('message-action-$actionId')))
              .dy,
      ];
      expect(actionTops, orderedEquals([...actionTops]..sort()));
      expect(
        find.byKey(const ValueKey('message-action-divider-utility')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('message-action-divider-destructive')),
        findsOneWidget,
      );

      await _dismissMessageActionsPopover(tester);
    });

    testWidgets('reduced motion presents the complete surface immediately', (
      tester,
    ) async {
      final prefs = await _mockPrefs();
      await _pumpMessageActionsPopover(
        tester,
        message: _message(),
        prefs: prefs,
        disableAnimations: true,
      );

      expect(
        find.byKey(const ValueKey('message-action-reaction-tray')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('message-action-preview')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('message-action-surface')),
        findsOneWidget,
      );
      await _dismissMessageActionsPopover(tester);
    });

    testWidgets('shows parity actions for a regular message', (tester) async {
      final prefs = await _mockPrefs();
      await _pumpSheet(tester, message: _message(), prefs: prefs);

      expect(find.text('Copy text'), findsOneWidget);
      expect(find.text('Copy link'), findsOneWidget);
      expect(find.text('Mark unread'), findsOneWidget);
      expect(find.text('Follow thread'), findsOneWidget);
      // No thread context → no Reply fast action.
      expect(find.text('Reply'), findsNothing);
      // No signing identity → no reminders; no manage rights → no edit/delete.
      expect(find.text('Remind me'), findsNothing);
      expect(find.text('Edit message'), findsNothing);
      expect(find.text('Delete message'), findsNothing);
      expect(find.byTooltip('Close sheet'), findsNothing);
      expect(find.byKey(const ValueKey('quick-reaction-more')), findsOneWidget);
      expect(
        find.byWidgetPredicate(
          (widget) =>
              widget.key is ValueKey<String> &&
              (widget.key! as ValueKey<String>).value.startsWith(
                'quick-reaction-',
              ),
        ),
        findsNWidgets(6),
      );
      expect(
        tester.getSize(find.byKey(const ValueKey('quick-reaction-\u{1F44D}'))),
        const Size.square(52),
      );
    });

    testWidgets('promotes Reply, Copy link, and Remind me to the fast-actions '
        'row', (tester) async {
      final prefs = await _mockPrefs();
      await _pumpSheet(
        tester,
        message: _message(),
        prefs: prefs,
        allMessages: [_message()],
        reminderService: _stubReminderService(),
      );

      expect(find.text('Reply'), findsOneWidget);
      expect(find.text('Copy link'), findsOneWidget);
      expect(find.text('Remind me'), findsOneWidget);
      // Promoted actions no longer appear under their old list-row labels.
      expect(find.text('Reply in thread'), findsNothing);
      expect(find.text('Remind me later'), findsNothing);
    });

    testWidgets('hides utility actions for system messages', (tester) async {
      final prefs = await _mockPrefs();
      await _pumpSheet(tester, message: _message(isSystem: true), prefs: prefs);

      expect(find.text('Copy text'), findsNothing);
      expect(find.text('Copy link'), findsNothing);
      expect(find.text('Mark unread'), findsNothing);
      expect(find.text('Follow thread'), findsNothing);
      expect(find.text('Reply'), findsNothing);
      expect(find.text('Remind me'), findsNothing);
      expect(find.byTooltip('Close sheet'), findsNothing);
      expect(
        find.byWidgetPredicate(
          (widget) =>
              widget.key is ValueKey<String> &&
              (widget.key! as ValueKey<String>).value.startsWith(
                'quick-reaction-',
              ),
        ),
        findsNWidgets(6),
      );
    });

    testWidgets('keeps six reaction targets within a narrow phone', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(375, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final prefs = await _mockPrefs();

      await _pumpSheet(tester, message: _message(), prefs: prefs);

      expect(
        find.byWidgetPredicate(
          (widget) =>
              widget.key is ValueKey<String> &&
              (widget.key! as ValueKey<String>).value.startsWith(
                'quick-reaction-',
              ),
        ),
        findsNWidgets(6),
      );
      expect(
        tester
            .getSize(find.byKey(const ValueKey('quick-reaction-\u{1F44D}')))
            .width,
        inInclusiveRange(44, 52),
      );
      expect(tester.takeException(), isNull);
    });

    testWidgets('keeps the reaction popover on-screen when neither side fits', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(320, 240);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      const anchorRect = Rect.fromLTWH(40, 60, 240, 100);
      const safeTop = 24.0;
      const visibleBottom = 200.0;
      const trayHeight = 68.0;
      const trayGap = Grid.xxs;
      final prefs = await _mockPrefs();
      expect(anchorRect.top - trayGap - trayHeight, lessThan(safeTop));
      expect(
        anchorRect.bottom + trayGap + trayHeight,
        greaterThan(visibleBottom),
      );

      await tester.pumpWidget(
        ProviderScope(
          overrides: [savedPrefsProvider.overrideWithValue(prefs)],
          child: MaterialApp(
            theme: AppTheme.light(),
            builder: (context, child) => MediaQuery(
              data: MediaQuery.of(context).copyWith(
                padding: const EdgeInsets.only(top: safeTop),
                viewInsets: const EdgeInsets.only(bottom: 40),
              ),
              child: child!,
            ),
            home: Scaffold(
              body: Consumer(
                builder: (context, ref, _) => TextButton(
                  onPressed: () => showMessageActions(
                    context: context,
                    ref: ref,
                    message: _message(isSystem: true),
                    channelId: _channelId,
                    canManageMessage: false,
                    anchorRect: anchorRect,
                  ),
                  child: const Text('open popover'),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('open popover'));
      await tester.pumpAndSettle();

      final trayRect = tester.getRect(
        find.byKey(const ValueKey('reaction-popover-tray')),
      );
      expect(trayRect.top, greaterThanOrEqualTo(safeTop));
      expect(trayRect.bottom, lessThanOrEqualTo(visibleBottom));
    });

    testWidgets('shows Edit/Delete only with manage rights', (tester) async {
      final prefs = await _mockPrefs();
      await _pumpSheet(
        tester,
        message: _message(),
        prefs: prefs,
        canManageMessage: true,
      );

      expect(find.text('Edit message'), findsOneWidget);
      expect(find.text('Delete message'), findsOneWidget);
    });

    testWidgets('Mark read appears for unread messages and advances the '
        'message marker', (tester) async {
      final prefs = await _mockPrefs();
      final notifier = _FakeReadStateNotifier(
        _readState(const {_channelId: 500}),
      );
      await _pumpSheet(
        tester,
        message: _message(createdAt: 900),
        prefs: prefs,
        readStateOverride: () => notifier,
      );

      expect(find.text('Mark read'), findsOneWidget);
      await tester.tap(find.text('Mark read'));
      await tester.pumpAndSettle();

      expect(notifier.markedRead, {'msg:msg-1': 900});
    });

    testWidgets('Mark unread forces the message unread', (tester) async {
      final prefs = await _mockPrefs();
      final notifier = _FakeReadStateNotifier(
        _readState(const {_channelId: 2000}),
      );
      await _pumpSheet(
        tester,
        message: _message(createdAt: 900),
        prefs: prefs,
        readStateOverride: () => notifier,
      );

      expect(find.text('Mark unread'), findsOneWidget);
      await tester.tap(find.text('Mark unread'));
      await tester.pumpAndSettle();

      // The force flag is message-scoped, mapped to its channel so tiles and
      // badges surface it.
      expect(notifier.markedUnread, ['msg:msg-1']);
      expect(notifier.state.forcedUnreadContexts, {'msg:msg-1': _channelId});
      expect(notifier.state.locallyForcedChannelIds, {_channelId});
    });

    testWidgets('Mark read after a forced unread clears the force flag so '
        'the toggle round-trips', (tester) async {
      final prefs = await _mockPrefs();
      final notifier = _FakeReadStateNotifier(
        _readState(const {_channelId: 2000}),
      );
      await _pumpSheet(
        tester,
        message: _message(createdAt: 900),
        prefs: prefs,
        readStateOverride: () => notifier,
      );

      // Force the message unread; the row flips to Mark read.
      await tester.tap(find.text('Mark unread'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('open'));
      await tester.pumpAndSettle();
      expect(find.text('Mark read'), findsOneWidget);

      // Mark read must clear the message's own force flag — otherwise the
      // row sticks on "Mark read".
      await tester.tap(find.text('Mark read'));
      await tester.pumpAndSettle();

      final container = ProviderScope.containerOf(
        tester.element(find.text('open')),
      );
      expect(container.read(readStateProvider).forcedUnreadContexts, isEmpty);

      await tester.tap(find.text('open'));
      await tester.pumpAndSettle();
      expect(find.text('Mark unread'), findsOneWidget);
      expect(find.text('Mark read'), findsNothing);
    });

    testWidgets('message-level Mark read leaves a channel-level forced '
        'unread untouched', (tester) async {
      final prefs = await _mockPrefs();
      final notifier = _FakeReadStateNotifier(
        ReadStateState(
          isReady: true,
          pubkey: 'self',
          contexts: const {_channelId: 2000},
          version: 1,
          // Channel forced unread from the channel tile, message forced
          // unread from this sheet.
          forcedUnreadContexts: const {
            _channelId: _channelId,
            'msg:msg-1': _channelId,
          },
        ),
      );
      await _pumpSheet(
        tester,
        message: _message(createdAt: 900),
        prefs: prefs,
        readStateOverride: () => notifier,
      );

      expect(find.text('Mark read'), findsOneWidget);
      await tester.tap(find.text('Mark read'));
      await tester.pumpAndSettle();

      // The message's flag is gone; the user's channel-level choice stays.
      final readState = notifier.state;
      expect(readState.forcedUnreadContexts, {_channelId: _channelId});
      expect(readState.locallyForcedChannelIds, {_channelId});
    });

    testWidgets('hides read-state row while read state is not ready', (
      tester,
    ) async {
      final prefs = await _mockPrefs();
      await _pumpSheet(
        tester,
        message: _message(),
        prefs: prefs,
        readStateOverride: () =>
            _FakeReadStateNotifier(_readState(const {}, isReady: false)),
      );

      expect(find.text('Mark unread'), findsNothing);
      expect(find.text('Mark read'), findsNothing);
    });

    testWidgets('Follow thread toggles the effective root id', (tester) async {
      final prefs = await _mockPrefs();
      await _pumpSheet(
        tester,
        message: _message(rootId: 'root-9'),
        prefs: prefs,
      );

      await tester.tap(find.text('Follow thread'));
      await tester.pumpAndSettle();

      final container = ProviderScope.containerOf(
        tester.element(find.text('open')),
      );
      expect(container.read(threadFollowsProvider).followedRootIds, {'root-9'});

      // Re-open: the row now offers Unfollow.
      await tester.tap(find.text('open'));
      await tester.pumpAndSettle();
      expect(find.text('Unfollow thread'), findsOneWidget);

      await tester.tap(find.text('Unfollow thread'));
      await tester.pumpAndSettle();
      expect(container.read(threadFollowsProvider).followedRootIds, isEmpty);
    });
  });

  group('showImageActions', () {
    testWidgets('labels the destructive action as deleting the message', (
      tester,
    ) async {
      await _pumpImageSheet(
        tester,
        message: _message(),
        canManageMessage: true,
      );

      expect(find.text('Delete message'), findsOneWidget);
      expect(find.text('Delete upload'), findsNothing);
    });
  });

  group('downloadedImageFilename', () {
    test('preserves gif file extensions', () {
      expect(
        downloadedImageFilename('https://example.com/animation.gif', null),
        'animation.gif',
      );
    });

    test('uses gif extension for gif content types', () {
      expect(
        downloadedImageFilename(
          'https://example.com/download',
          'image/gif; charset=binary',
        ),
        matches(RegExp(r'^buzz-\d+\.gif$')),
      );
    });
  });

  group('messageLinkFor', () {
    test('builds a canonical link with thread context', () {
      expect(
        messageLinkFor(
          message: _message(rootId: 'root-1'),
          channelId: _channelId,
        ),
        'buzz://message?channel=chan-1&id=msg-1&thread=root-1',
      );
    });

    test('omits thread for top-level messages', () {
      expect(
        messageLinkFor(message: _message(), channelId: _channelId),
        'buzz://message?channel=chan-1&id=msg-1',
      );
    });
  });
}
