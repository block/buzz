import 'package:buzz/shared/read_state/thread_follows_format.dart';
import 'package:buzz/features/channels/thread_follows/thread_follows_storage.dart';
import 'package:buzz/shared/widgets/sheet_action_section.dart';
import 'package:buzz/shared/widgets/app_list_card.dart';
import 'dart:ui' as ui;
import 'dart:convert';
import 'dart:io';
import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji_provider.dart';
import 'package:buzz/shared/emoji/emoji_data.dart';
import 'package:buzz/shared/emoji/emoji_picker.dart';
import 'package:buzz/shared/emoji/emoji_burst.dart';
import 'package:buzz/shared/emoji/recent_emoji_provider.dart';
import 'package:flutter/services.dart';
import 'package:buzz/shared/emoji/emoji_data_provider.dart';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/message_actions.dart';
import 'package:buzz/features/channels/message_long_press_region.dart';
import 'package:buzz/features/channels/message_reply_drag_target.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/features/channels/thread_follows/thread_follows_provider.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/reminders/reminder_service.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart' show Override;
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:photo_manager/photo_manager.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/thread_ids.dart';

part 'message_actions_row_rebuild_tests.dart';
part 'message_actions_scroll_journeys.dart';
part 'message_actions_test_helpers.dart';

const _channelId = 'chan-1';

void main() {
  _messageActionsScrollTests();
  _messageActionsRowRebuildTests();
  testWidgets(
    'quick reactions show selection, label controls, and remove our reaction',
    (tester) async {
      final semantics = tester.ensureSemantics();
      final actions = _RecordingReactionActions();
      await _pumpMessageActionsPopover(
        tester,
        message: _message(
          reactions: [
            const TimelineReaction(
              emoji: '👍',
              count: 1,
              reactedByCurrentUser: true,
              userPubkeys: ['self'],
              currentUserReactionId: 'our-reaction',
            ),
          ],
        ),
        prefs: await _mockPrefs(),
        createChannelActions: (_) => actions,
      );
      final emoji = find.byKey(const ValueKey('quick-reaction-👍'));
      final owner = tester.widget<Semantics>(
        find.descendant(of: emoji, matching: find.byType(Semantics)).first,
      );
      expect(owner.properties.button, isTrue);
      expect(owner.properties.selected, isTrue);
      expect(owner.properties.label, isNotEmpty);
      expect(owner.properties.onTap, isNotNull);
      expect(find.bySemanticsLabel('Add reaction'), findsOneWidget);
      await tester.tap(emoji);
      await tester.pumpAndSettle();
      expect(actions.removed, ['our-reaction']);
      expect(actions.added, isEmpty);
      semantics.dispose();
    },
  );

  testWidgets('two active pointers select only one sheet action', (
    tester,
  ) async {
    final actions = _RecordingReactionActions();
    await _pumpMessageActionsPopover(
      tester,
      message: _message(),
      prefs: await _mockPrefs(),
      launcherOnNestedRoute: true,
      createChannelActions: (_) => actions,
    );
    final first = await tester.startGesture(
      tester.getCenter(find.byKey(const ValueKey('quick-reaction-👍'))),
      pointer: 1,
    );
    final second = await tester.startGesture(
      tester.getCenter(find.text('Copy text')),
      pointer: 2,
    );
    await first.up();
    await second.up();
    await tester.pumpAndSettle();
    expect(actions.added, ['👍']);
    expect(actions.targetAuthors, ['alice']);
    expect(
      find.byKey(const ValueKey('message-actions-underlying-page')),
      findsOneWidget,
    );
  });

  // Some sheets build the app lifecycle, which listens for network changes.
  // That listen is asynchronous, so a missing plugin fails whichever test
  // happens to be running. Flutter documents mock handlers as cleared after
  // each test, so install it before every test.
  setUp(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('dev.fluttercommunity.plus/connectivity_status'),
          (_) async => null,
        );
  });

  testWidgets(
    'message long press keeps taps and delayed scrolling while repeated holds win',
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
      await tester.pump(const Duration(milliseconds: 300));
      expect(longPresses, 5);
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
    variant: TargetPlatformVariant({
      TargetPlatform.iOS,
      TargetPlatform.android,
    }),
  );

  testWidgets('iOS message long press recognizes at 500 ms', (tester) async {
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
      await tester.pump(const Duration(milliseconds: 499));
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

  testWidgets('message snapshot bounds very tall raster dimensions', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(800, 5000);
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.view.resetPhysicalSize);
    MessageLongPressDetails? longPressDetails;
    ui.Image? snapshot;
    addTearDown(() => snapshot?.dispose());

    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Material(
            child: MessageLongPressInkWell(
              key: const ValueKey('tall-snapshot-gesture-target'),
              onLongPressDetails: (details) => longPressDetails = details,
              child: const SizedBox(width: 240, height: 4096),
            ),
          ),
        ),
      ),
    );

    await tester.longPress(
      find.byKey(const ValueKey('tall-snapshot-gesture-target')),
    );
    expect(longPressDetails, isNotNull);

    final capture = longPressDetails!.captureSnapshot();
    await tester.pump();
    snapshot = await capture;

    expect(snapshot.width, 120);
    expect(snapshot.height, 2048);
  });

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

  group('targets-only message long press', () {
    Widget region({
      required ValueNotifier<double> bubbleWidth,
      required VoidCallback onLongPress,
      required VoidCallback onTap,
    }) {
      return MaterialApp(
        home: Scaffold(
          body: ListView(
            children: [
              Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  const Text('Date'),
                  Material(
                    child: MessageLongPressInkWell(
                      targetsOnly: true,
                      onTap: onTap,
                      onLongPress: (_) => onLongPress(),
                      child: ValueListenableBuilder<double>(
                        valueListenable: bubbleWidth,
                        builder: (context, width, _) => Align(
                          alignment: Alignment.centerLeft,
                          child: MessageLongPressTarget(
                            child: SizedBox(
                              width: width,
                              height: 40,
                              child: const Text('Hi'),
                            ),
                          ),
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            ],
          ),
        ),
      );
    }

    Future<void> holdWhileResizing(
      WidgetTester tester,
      ValueNotifier<double> bubbleWidth,
      double resizedWidth,
    ) async {
      final gesture = await tester.startGesture(
        tester.getTopLeft(find.text('Hi')) + const Offset(150, 20),
      );
      await tester.pump(const Duration(milliseconds: 100));
      bubbleWidth.value = resizedWidth;
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 600));
      await gesture.up();
      await tester.pump();
    }

    testWidgets('a hold that started on the bubble opens after it shrinks', (
      tester,
    ) async {
      var longPresses = 0;
      var taps = 0;
      final bubbleWidth = ValueNotifier<double>(200);
      addTearDown(bubbleWidth.dispose);
      await tester.pumpWidget(
        region(
          bubbleWidth: bubbleWidth,
          onLongPress: () => longPresses += 1,
          onTap: () => taps += 1,
        ),
      );

      await holdWhileResizing(tester, bubbleWidth, 100);

      expect(longPresses, 1);
      expect(taps, 0);
    });

    testWidgets(
      'a hold that started beside the bubble stays inert after it widens',
      (tester) async {
        var longPresses = 0;
        var taps = 0;
        final bubbleWidth = ValueNotifier<double>(100);
        addTearDown(bubbleWidth.dispose);
        await tester.pumpWidget(
          region(
            bubbleWidth: bubbleWidth,
            onLongPress: () => longPresses += 1,
            onTap: () => taps += 1,
          ),
        );

        await holdWhileResizing(tester, bubbleWidth, 200);

        expect(longPresses, 0);
        expect(taps, 0);
      },
    );

    testWidgets(
      'the message node has the only screen-reader long press and it opens',
      (tester) async {
        var longPresses = 0;
        final bubbleWidth = ValueNotifier<double>(100);
        addTearDown(bubbleWidth.dispose);
        final semantics = tester.ensureSemantics();
        await tester.pumpWidget(
          region(
            bubbleWidth: bubbleWidth,
            onLongPress: () => longPresses += 1,
            onTap: () {},
          ),
        );

        final longPressNodes = <SemanticsNode>[];
        void collect(SemanticsNode node) {
          if (node.getSemanticsData().hasAction(SemanticsAction.longPress)) {
            longPressNodes.add(node);
          }
          node.visitChildren((child) {
            collect(child);
            return true;
          });
        }

        collect(
          tester
              .binding
              .renderViews
              .single
              .owner!
              .semanticsOwner!
              .rootSemanticsNode!,
        );

        expect(longPressNodes, hasLength(1));
        expect(longPressNodes.single.getSemanticsData().label, 'Hi');
        tester.binding.renderViews.single.owner!.semanticsOwner!.performAction(
          longPressNodes.single.id,
          SemanticsAction.longPress,
        );
        await tester.pump();
        expect(longPresses, 1);
        semantics.dispose();
      },
    );
  });

  group('showMessageActions', () {
    for (final platform in [TargetPlatform.iOS, TargetPlatform.android]) {
      testWidgets('uses sectioned sheet on $platform even with a snapshot', (
        tester,
      ) async {
        debugDefaultTargetPlatformOverride = platform;
        addTearDown(() => debugDefaultTargetPlatformOverride = null);
        var captures = 0;
        final harness = await _pumpMessageActionsPopover(
          tester,
          message: _message(),
          prefs: await _mockPrefs(),
          canManageMessage: true,
          allMessages: [_message()],
          reminderService: _stubReminderService(),
          captureAnchorSnapshot: () async {
            captures++;
            return _testMessageSnapshot();
          },
        );
        expect(find.byType(BottomSheet), findsOneWidget);
        expect(find.byType(SheetActionSection), findsNWidgets(3));
        expect(find.byType(AppListCard), findsNWidgets(3));
        expect(captures, 0);
        expect(harness.sourceHidden.value, isFalse);
        for (final label in [
          'Follow thread',
          'Copy text',
          'Edit message',
          'Delete message',
        ]) {
          expect(
            find.ancestor(
              of: find.text(label),
              matching: find.byType(SheetActionSection),
            ),
            findsOneWidget,
          );
        }
        await tester.pumpWidget(const SizedBox());
        debugDefaultTargetPlatformOverride = null;
      });
    }

    testWidgets('sheet actions grow with large text and remain scrollable', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(375, 667);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      await _pumpMessageActionsPopover(
        tester,
        message: _message(),
        prefs: await _mockPrefs(),
        canManageMessage: true,
        allMessages: [_message()],
        reminderService: _stubReminderService(),
        textScaler: TextScaler.linear(2),
      );
      await tester.ensureVisible(find.text('Delete message'));
      expect(find.text('Delete message').hitTestable(), findsOneWidget);
      expect(tester.takeException(), isNull);
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

    testWidgets(
      'places four quick action tiles between reactions and sections',
      (tester) async {
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
        final reply = tester.getCenter(find.text('Reply'));
        final link = tester.getCenter(find.text('Copy link'));
        final remind = tester.getCenter(find.text('Remind me'));
        final mark = tester.getCenter(find.text('Mark unread'));
        expect(mark.dx, lessThan(reply.dx));
        expect(mark.dy, reply.dy);
        expect(reply.dy, link.dy);
        expect(link.dy, remind.dy);
        expect(reply.dx, lessThan(link.dx));
        expect(link.dx, lessThan(remind.dx));
        expect(
          tester
              .getBottomLeft(find.byKey(const ValueKey('quick-reaction-more')))
              .dy,
          lessThan(reply.dy),
        );
        expect(
          remind.dy,
          lessThan(tester.getTopLeft(find.byType(SheetActionSection).first).dy),
        );
        for (final label in [
          'Mark unread',
          'Reply',
          'Copy link',
          'Remind me',
        ]) {
          expect(
            find.ancestor(
              of: find.text(label),
              matching: find.byType(SheetActionSection),
            ),
            findsNothing,
          );
        }
        // Keep the existing labels.
        expect(find.text('Reply in thread'), findsNothing);
        expect(find.text('Remind me later'), findsNothing);
      },
    );

    for (final action in [
      null,
      'Copy link',
      'Copy text',
      'Follow thread',
      'Mark unread',
    ]) {
      testWidgets('composer focus after sheet dismissal: $action', (
        tester,
      ) async {
        final focusNode = FocusNode();
        addTearDown(focusNode.dispose);
        await _pumpMessageActionsPopover(
          tester,
          message: _message(),
          prefs: await _mockPrefs(),
          composerFocusNode: focusNode,
          composerInitiallyFocused: true,
        );
        expect(focusNode.hasFocus, isFalse);
        if (action == null) {
          await tester.tapAt(const Offset(10, 10));
        } else {
          await tester.tap(find.text(action));
        }
        await tester.pumpAndSettle();
        expect(focusNode.hasFocus, action == null);
      });
    }

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

    testWidgets('system messages use a dismissible reaction sheet', (
      tester,
    ) async {
      await _pumpSheet(
        tester,
        message: _message(isSystem: true),
        prefs: await _mockPrefs(),
        anchorRect: const Rect.fromLTWH(20, 20, 100, 100),
      );
      expect(find.byType(BottomSheet), findsOneWidget);
      expect(find.byKey(const ValueKey('quick-reaction-more')), findsOneWidget);
      await tester.tapAt(const Offset(10, 10));
      await tester.pumpAndSettle();
      expect(find.byType(BottomSheet), findsNothing);
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

    // A signed-in user may have no published profile, so the caller passes
    // a null currentPubkey. Catch-up must still classify mentions with the
    // signing key, like the badge and channel list do.
    for (final (label, tags, expected) in [
      ('an ordinary message', const <List<String>>[], 'Mark unread'),
      (
        'a mention',
        const [
          ['p', 'self'],
        ],
        'Mark read',
      ),
    ]) {
      testWidgets('without a profile, channel catch-up reads $label', (
        tester,
      ) async {
        await _pumpSheet(
          tester,
          message: _message(createdAt: 900, tags: tags),
          prefs: await _mockPrefs(),
          readStateOverride: () => _FakeReadStateNotifier(
            _readState(const {_channelId: 500, 'activity:$_channelId': 2000}),
          ),
          currentPubkey: null,
          listChannel: true,
        );

        expect(find.text(expected), findsOneWidget);
      });
    }

    testWidgets('the anchored sheet uses channel catch-up without a profile', (
      tester,
    ) async {
      await _pumpMessageActionsPopover(
        tester,
        message: _message(createdAt: 900),
        prefs: await _mockPrefs(),
        readStateOverride: () => _FakeReadStateNotifier(
          _readState(const {_channelId: 500, 'activity:$_channelId': 2000}),
        ),
        currentPubkey: null,
        listChannel: true,
      );

      expect(find.text('Mark unread'), findsOneWidget);
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
        message: _message(rootId: threadIdRootDash9),
        prefs: prefs,
      );

      await tester.tap(find.text('Follow thread'));
      await tester.pumpAndSettle();

      final container = ProviderScope.containerOf(
        tester.element(find.text('open')),
      );
      expect(container.read(threadFollowsProvider).followedRootIds, {
        threadIdRootDash9,
      });

      // Re-open: the row now offers Unfollow.
      await tester.tap(find.text('open'));
      await tester.pumpAndSettle();
      expect(find.text('Unfollow thread'), findsOneWidget);

      await tester.tap(find.text('Unfollow thread'));
      await tester.pumpAndSettle();
      expect(container.read(threadFollowsProvider).followedRootIds, isEmpty);
    });
  });

  test('an unimported pre-sync follow still shows, and a synced choice '
      'wins', () async {
    final prefs = await _mockPrefs();
    final legacyRoot = 'b' * 64;
    final syncedRoot = 'c' * 64;
    ProviderContainer open(Map<String, ThreadFollowChoice> synced) {
      final container = ProviderContainer(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          myPubkeyProvider.overrideWithValue('self'),
          readStateProvider.overrideWith(
            () => _FakeReadStateNotifier(
              ReadStateState(
                isReady: true,
                pubkey: 'self',
                contexts: const {},
                version: 1,
                threadFollows: synced,
              ),
            ),
          ),
        ],
      );
      addTearDown(container.dispose);
      return container;
    }

    final relayUrl = open(const {}).read(relayConfigProvider).baseUrl;
    ThreadFollowsStorage(prefs).write('self', relayUrl, [
      ThreadFollowEntry(rootId: legacyRoot, followedAt: 1),
      ThreadFollowEntry(rootId: syncedRoot, followedAt: 1),
    ]);
    final follows = open({
      threadFollowKey(_channelId, syncedRoot): (follow: false, at: 2),
    }).read(threadFollowsProvider);
    expect(follows.followedRootIds, {legacyRoot});
    expect(follows.unfollowedRootIds, {syncedRoot});
    // A synced choice names its channel; the pre-sync one has none known.
    expect(follows.channelByRoot, {syncedRoot: _channelId});
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
      expect(find.byType(AppListCard), findsNWidgets(2));
      expect(
        find.descendant(
          of: find.byType(SheetActionSection),
          matching: find.byType(ListTile),
        ),
        findsNWidgets(4),
      );
    });

    testWidgets('saving on iOS reports the result of the add-only write', (
      tester,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      try {
        final messenger =
            TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
        final permissionRequests = <Object?>[];
        // Only the permission request is answered: add-only access cannot read
        // the library, so any other photo_manager call fails as it would there.
        const photoManager = MethodChannel('com.fluttercandies/photo_manager');
        const saver = MethodChannel('buzz/photo_library_saver');
        addTearDown(() {
          messenger.setMockMethodCallHandler(photoManager, null);
          messenger.setMockMethodCallHandler(saver, null);
        });
        messenger.setMockMethodCallHandler(photoManager, (call) async {
          if (call.method != 'requestPermissionExtend') {
            throw PlatformException(code: 'add_only_cannot_read');
          }
          permissionRequests.add(call.arguments);
          return PermissionState.authorized.index;
        });
        final saves = <Map<Object?, Object?>>[];
        PlatformException? saveFailure;
        messenger.setMockMethodCallHandler(saver, (call) async {
          expect(call.method, 'saveImage');
          saves.add(call.arguments as Map<Object?, Object?>);
          if (saveFailure case final failure?) throw failure;
          return null;
        });

        await _pumpImageSheet(
          tester,
          message: _message(),
          mediaClient: MockClient(
            (_) async => http.Response.bytes(
              [1, 2, 3],
              200,
              headers: {'content-type': 'image/png'},
            ),
          ),
        );
        await tester.tap(find.text('Save image'));
        await tester.pumpAndSettle();

        expect(find.text('Saved to Photos'), findsOneWidget);
        expect(saves.single, {
          'bytes': Uint8List.fromList([1, 2, 3]),
          'filename': 'photo.png',
        });
        expect(
          (permissionRequests.single
              as Map<Object?, Object?>)['iosAccessLevel'],
          IosAccessLevel.addOnly.index + 1,
        );

        saveFailure = PlatformException(code: 'save_failed');
        tester
            .state<ScaffoldMessengerState>(find.byType(ScaffoldMessenger))
            .removeCurrentSnackBar();
        await tester.tap(find.text('open image actions'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Save image'));
        await tester.pumpAndSettle();

        expect(find.text('Could not save image'), findsOneWidget);
        expect(saves, hasLength(2));
      } finally {
        debugDefaultTargetPlatformOverride = null;
      }
    });
  });

  testWidgets('image sheet omits message deletion without permission', (
    tester,
  ) async {
    await _pumpImageSheet(tester, message: _message(), canManageMessage: false);
    expect(find.byType(AppListCard), findsOneWidget);
    expect(
      find.descendant(
        of: find.byType(SheetActionSection),
        matching: find.byType(ListTile),
      ),
      findsNWidgets(3),
    );
    expect(find.text('Delete message'), findsNothing);
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
          message: _message(rootId: threadIdRootDash1),
          channelId: _channelId,
        ),
        'buzz://message?channel=chan-1&id=msg-1&thread=$threadIdRootDash1',
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
