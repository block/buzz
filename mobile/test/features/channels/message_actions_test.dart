import 'package:buzz/shared/widgets/sheet_action_section.dart';
import 'package:buzz/shared/widgets/app_list_card.dart';
import 'dart:async';
import 'dart:ui' as ui;
import 'package:flutter/services.dart';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/message_actions.dart';
import 'package:buzz/features/channels/reaction_row.dart';
import 'package:buzz/features/channels/message_long_press_region.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/features/channels/thread_follows/thread_follows_provider.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/reminders/reminder_service.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:buzz/shared/widgets/native_message_presentation.dart';
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
  List<List<String>> tags = const [],
}) => TimelineMessage(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  content: 'hello world',
  isSystem: isSystem,
  rootId: rootId,
  tags: tags,
);

final _listedChannel = Channel(
  id: _channelId,
  name: 'general',
  channelType: 'stream',
  visibility: 'open',
  description: '',
  createdBy: 'creator',
  createdAt: DateTime(2026),
  memberCount: 2,
  isMember: true,
);

class _ListedChannelsNotifier extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [_listedChannel];
}

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
  Rect? anchorRect,
  bool nativePresentation = false,
  String? currentPubkey = 'self',
  bool listChannel = false,
}) async {
  Future<void>? presentation;
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
        if (listChannel)
          channelsProvider.overrideWith(_ListedChannelsNotifier.new),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: Consumer(
            builder: (context, ref, _) {
              // The channel page keeps the channel list alive.
              if (listChannel) ref.watch(channelsProvider);
              return TextButton(
                onPressed: () => presentation = showMessageActions(
                  context: context,
                  ref: ref,
                  message: message,
                  channelId: _channelId,
                  canManageMessage: canManageMessage,
                  anchorRect: anchorRect,
                  captureAnchorSnapshot: nativePresentation
                      ? _testMessageSnapshot
                      : null,
                  allMessages: allMessages,
                  currentPubkey: currentPubkey,
                  isMember: true,
                ),
                child: const Text('open'),
              );
            },
          ),
        ),
      ),
    ),
  );
  if (listChannel) await tester.pump();
  if (nativePresentation) {
    await tester.runAsync(() async {
      await tester.tap(find.text('open'));
      await presentation;
    });
  } else {
    await tester.tap(find.text('open'));
  }
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
  TextScaler textScaler = TextScaler.noScaling,
  Future<ui.Image> Function()? captureAnchorSnapshot,
  FocusNode? composerFocusNode,
  bool composerInitiallyFocused = false,
  bool launcherOnNestedRoute = false,
  ChannelActions Function(Ref ref)? createChannelActions,
  Rect anchorRect = const Rect.fromLTWH(32, 260, 300, 72),
  String? currentPubkey = 'self',
  bool listChannel = false,
}) async {
  final sourceHidden = ValueNotifier(false);

  Widget launcherPage() => Scaffold(
    key: const ValueKey('message-actions-underlying-page'),
    body: Consumer(
      builder: (context, ref, _) {
        // The channel page keeps the channel list alive.
        if (listChannel) ref.watch(channelsProvider);
        return Column(
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
                currentPubkey: currentPubkey,
                isMember: true,
                anchorRect: anchorRect,
                captureAnchorSnapshot:
                    captureAnchorSnapshot ?? _testMessageSnapshot,
                onPopoverPreviewVisibilityChanged: (visible) =>
                    sourceHidden.value = visible,
                onPopoverDismissed: () => sourceHidden.value = false,
                composerFocusNode: composerFocusNode,
                restoreComposerFocus: composerFocusNode?.requestFocus,
              ),
              child: const Text('open message actions'),
            ),
          ],
        );
      },
    ),
  );

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
        if (createChannelActions != null)
          channelActionsProvider.overrideWith(createChannelActions),
        if (listChannel)
          channelsProvider.overrideWith(_ListedChannelsNotifier.new),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(context).copyWith(
            disableAnimations: disableAnimations,
            viewInsets: viewInsets,
            textScaler: textScaler,
          ),
          child: child!,
        ),
        home: launcherOnNestedRoute
            ? Builder(
                builder: (context) => Scaffold(
                  key: const ValueKey('message-actions-root-page'),
                  body: TextButton(
                    key: const ValueKey('push-message-actions-launcher'),
                    onPressed: () => Navigator.of(context).push(
                      MaterialPageRoute<void>(builder: (_) => launcherPage()),
                    ),
                    child: const Text('push launcher'),
                  ),
                ),
              )
            : launcherPage(),
      ),
    ),
  );
  if (launcherOnNestedRoute) {
    await tester.tap(
      find.byKey(const ValueKey('push-message-actions-launcher')),
    );
    await tester.pumpAndSettle();
  }
  if (composerInitiallyFocused) {
    composerFocusNode!.requestFocus();
    await tester.pump();
  }
  if (listChannel) await tester.pump();
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

class _FakeChannelActions extends ChannelActions {
  final reactions = <({String eventId, String emoji})>[];

  _FakeChannelActions(Ref ref)
    : super(
        ref: ref,
        session: ref.read(relaySessionProvider.notifier),
        signedEventRelay: SignedEventRelay(
          session: ref.read(relaySessionProvider.notifier),
          nsec: null,
        ),
        currentPubkey: 'self',
      );

  @override
  Future<void> addReaction(String eventId, String emoji) async {
    reactions.add((eventId: eventId, emoji: emoji));
  }
}

void main() {
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

  testWidgets(
    'native menu respects ownership and dispatches the target read action',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      Map<Object?, Object?>? payload;
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        NativeMessagePresentation.channel,
        (call) async {
          if (call.method == 'supportsMessage') return {'supported': true};
          payload = call.arguments as Map<Object?, Object?>;
          return {'action': 'read'};
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          NativeMessagePresentation.channel,
          null,
        ),
      );
      final notifier = _FakeReadStateNotifier(
        _readState(const {_channelId: 100000}),
      );
      await _pumpSheet(
        tester,
        message: _message(),
        prefs: await _mockPrefs(),
        readStateOverride: () => notifier,
        anchorRect: const Rect.fromLTWH(20, 200, 300, 80),
        nativePresentation: true,
      );
      expect(payload?['previewBytes'], isA<Uint8List>());
      expect(notifier.markedUnread, ['msg:msg-1']);
      final actions = (payload!['actions'] as List)
          .cast<Map<Object?, Object?>>();
      expect(actions.map((a) => a['id']), isNot(contains('edit')));
      expect(actions.map((a) => a['id']), isNot(contains('delete')));
      expect(find.text('Copy text'), findsNothing);
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets('the native menu uses channel catch-up without a profile', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    Map<Object?, Object?>? payload;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      NativeMessagePresentation.channel,
      (call) async {
        if (call.method == 'supportsMessage') return {'supported': true};
        payload = call.arguments as Map<Object?, Object?>;
        return <String, Object?>{};
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        NativeMessagePresentation.channel,
        null,
      ),
    );
    await _pumpSheet(
      tester,
      message: _message(createdAt: 900),
      prefs: await _mockPrefs(),
      readStateOverride: () => _FakeReadStateNotifier(
        _readState(const {_channelId: 500, 'activity:$_channelId': 2000}),
      ),
      anchorRect: const Rect.fromLTWH(20, 200, 300, 80),
      nativePresentation: true,
      currentPubkey: null,
      listChannel: true,
    );
    final actions = (payload!['actions'] as List).cast<Map<Object?, Object?>>();
    expect(
      actions.firstWhere((a) => a['id'] == 'read')['title'],
      'Mark unread',
    );
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('system messages use the native tray without action rows', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    Map<Object?, Object?>? payload;
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      NativeMessagePresentation.channel,
      (call) async {
        if (call.method == 'supportsMessage') return {'supported': true};
        payload = call.arguments as Map<Object?, Object?>;
        return <String, Object?>{};
      },
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        NativeMessagePresentation.channel,
        null,
      ),
    );
    await _pumpSheet(
      tester,
      message: _message(isSystem: true),
      prefs: await _mockPrefs(),
      anchorRect: const Rect.fromLTWH(20, 200, 300, 80),
      nativePresentation: true,
    );
    expect(payload?['actions'], isEmpty);
    expect(payload?['reactions'], isNotEmpty);
    expect(payload?['previewBytes'], isA<Uint8List>());
    expect(find.byType(BottomSheet), findsNothing);
    debugDefaultTargetPlatformOverride = null;
  });

  for (final selectedAction in <String?>[
    null,
    'follow',
    'unmount',
    'unmountFailure',
    'overlap',
  ]) {
    testWidgets('native focus and source lifecycle for $selectedAction', (
      tester,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final focus = FocusNode();
      late Completer<void> ready;
      late Completer<Map<String, Object?>> result;
      await tester.runAsync(() async {
        ready = Completer<void>();
        result = Completer<Map<String, Object?>>();
      });
      final visibility = <bool>[];
      var dismissed = 0;
      var focusRestores = 0;
      late BuildContext pageContext;
      late WidgetRef pageRef;
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        NativeMessagePresentation.channel,
        (call) async {
          if (call.method == 'supportsMessage') return {'supported': true};
          expect(visibility, isEmpty);
          final args = call.arguments as Map;
          await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
            NativeMessagePresentation.channel.name,
            const StandardMethodCodec().encodeMethodCall(
              MethodCall('messagePresented', args['requestId']),
            ),
            (_) {},
          );
          ready.complete();
          return result.future;
        },
      );
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            savedPrefsProvider.overrideWithValue(await _mockPrefs()),
            myPubkeyProvider.overrideWithValue('self'),
            readStateProvider.overrideWith(
              () => _FakeReadStateNotifier(
                _readState(const {_channelId: 100000}),
              ),
            ),
            reminderServiceProvider.overrideWithValue(null),
          ],
          child: MaterialApp(
            home: Consumer(
              builder: (context, ref, _) {
                pageContext = context;
                pageRef = ref;
                return Scaffold(body: TextField(focusNode: focus));
              },
            ),
          ),
        ),
      );
      focus.requestFocus();
      await tester.pump();
      late Future<void> presentation;
      Future<void> openMenu() => showMessageActions(
        context: pageContext,
        ref: pageRef,
        message: _message(),
        channelId: _channelId,
        canManageMessage: false,
        anchorRect: const Rect.fromLTWH(20, 200, 300, 80),
        captureAnchorSnapshot: _testMessageSnapshot,
        composerFocusNode: focus,
        restoreComposerFocus: () {
          focusRestores++;
          focus.requestFocus();
        },
        onPopoverPreviewVisibilityChanged: visibility.add,
        onPopoverDismissed: () => dismissed++,
      );
      await tester.runAsync(() async {
        presentation = openMenu();
        if (selectedAction == 'overlap') {
          // A is awaiting preflight/capture, so both gestures could see focus.
          await openMenu();
          expect(focusRestores, 0);
        }
        await ready.future;
      });
      await tester.pump();
      expect(focus.hasFocus, isFalse);
      expect(visibility, [true]);
      if (selectedAction == 'overlap') {
        await tester.runAsync(openMenu);
        await tester.pump();
        expect(focus.hasFocus, isFalse);
        expect(focusRestores, 0);
      }
      if (selectedAction?.startsWith('unmount') == true) {
        await tester.pumpWidget(const SizedBox.shrink());
      }
      await tester.runAsync(() async {
        if (selectedAction == 'unmountFailure') {
          result.completeError(PlatformException(code: 'presentation-failed'));
        } else {
          result.complete({
            if (selectedAction == 'follow') 'action': selectedAction,
          });
        }
        await presentation;
      });
      await tester.pump();
      final restores = selectedAction == null || selectedAction == 'overlap';
      expect(focus.hasFocus, restores);
      expect(focusRestores, restores ? 1 : 0);
      expect(visibility, [true, false]);
      expect(dismissed, selectedAction?.startsWith('unmount') == true ? 0 : 1);
      await tester.pumpWidget(const SizedBox.shrink());
      focus.dispose();
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        NativeMessagePresentation.channel,
        null,
      );
      debugDefaultTargetPlatformOverride = null;
    });
  }

  for (final (reactionFirst, missingSnapshot) in [
    (false, false),
    (true, false),
    (true, true),
  ]) {
    testWidgets(
      'cross-surface native ownership reactionFirst=$reactionFirst missingSnapshot=$missingSnapshot',
      (tester) async {
        debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
        final focus = FocusNode();
        late Completer<Map<String, Object?>> result;
        late Completer<Map<String, Object?>> support;
        late Completer<void> entered;
        late Completer<void> ready;
        await tester.runAsync(() async {
          result = Completer();
          support = Completer();
          entered = Completer();
          ready = Completer();
        });
        var messages = 0;
        var reactions = 0;
        var restores = 0;
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          NativeMessagePresentation.channel,
          (call) async {
            if (call.method == 'supportsMessage') {
              entered.complete();
              return support.future;
            }
            if (call.method == 'message') messages++;
            if (call.method == 'reactions') reactions++;
            ready.complete();
            return result.future;
          },
        );
        late BuildContext pageContext;
        late WidgetRef pageRef;
        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              savedPrefsProvider.overrideWithValue(await _mockPrefs()),
              myPubkeyProvider.overrideWithValue('self'),
              readStateProvider.overrideWith(
                () => _FakeReadStateNotifier(
                  _readState(const {_channelId: 100000}),
                ),
              ),
              reminderServiceProvider.overrideWithValue(null),
            ],
            child: MaterialApp(
              home: Consumer(
                builder: (context, ref, _) {
                  pageContext = context;
                  pageRef = ref;
                  return Scaffold(
                    body: TextField(focusNode: focus, showCursor: false),
                  );
                },
              ),
            ),
          ),
        );
        focus.requestFocus();
        await tester.pump();
        Future<void> message() => showMessageActions(
          context: pageContext,
          ref: pageRef,
          message: _message(),
          channelId: _channelId,
          canManageMessage: false,
          anchorRect: const Rect.fromLTWH(20, 200, 300, 80),
          captureAnchorSnapshot: missingSnapshot ? null : _testMessageSnapshot,
          composerFocusNode: focus,
          restoreComposerFocus: () {
            restores++;
            focus.requestFocus();
          },
        );
        Future<void> reaction() => showReactionDetailSheet(
          context: pageContext,
          channelId: _channelId,
          reactions: const [
            TimelineReaction(
              emoji: '❤️',
              count: 1,
              reactedByCurrentUser: false,
              userPubkeys: [],
            ),
          ],
          initialEmoji: '❤️',
        );
        late Future<void> owner;
        await tester.runAsync(() async {
          if (reactionFirst) {
            owner = reaction();
            await ready.future;
            await message();
          } else {
            owner = message();
            await entered.future;
            await reaction();
            expect(reactions, 0);
            support.complete({'supported': true});
            await ready.future;
          }
        });
        await tester.pump();
        expect(messages, reactionFirst ? 0 : 1);
        expect(reactions, reactionFirst ? 1 : 0);
        expect(restores, 0);
        expect(focus.hasFocus, reactionFirst);
        expect(find.byType(BottomSheet), findsNothing);
        await tester.runAsync(() async {
          result.complete({});
          await owner;
        });
        await tester.pumpAndSettle();
        expect(focus.hasFocus, isTrue);
        expect(restores, reactionFirst ? 0 : 1);
        await tester.pumpWidget(const SizedBox.shrink());
        focus.dispose();
        tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          NativeMessagePresentation.channel,
          null,
        );
        debugDefaultTargetPlatformOverride = null;
      },
    );
  }

  testWidgets('native dismissal does not open the Flutter sheet', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      NativeMessagePresentation.channel,
      (call) async => call.method == 'supportsMessage'
          ? {'supported': true}
          : <String, Object?>{},
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        NativeMessagePresentation.channel,
        null,
      ),
    );
    await _pumpSheet(
      tester,
      message: _message(),
      prefs: await _mockPrefs(),
      anchorRect: const Rect.fromLTWH(20, 200, 300, 80),
      nativePresentation: true,
    );
    expect(find.text('Copy text'), findsNothing);
    expect(find.text('open'), findsOneWidget);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('unavailable native presentation falls back to message actions', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      NativeMessagePresentation.channel,
      (_) async => null,
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        NativeMessagePresentation.channel,
        null,
      ),
    );
    await _pumpSheet(
      tester,
      message: _message(),
      prefs: await _mockPrefs(),
      canManageMessage: true,
      anchorRect: const Rect.fromLTWH(20, 200, 300, 80),
    );
    expect(find.text('Edit message'), findsOneWidget);
    expect(find.text('Delete message'), findsOneWidget);
    debugDefaultTargetPlatformOverride = null;
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
          tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            NativeMessagePresentation.channel,
            (_) async => null,
          );
          addTearDown(
            () =>
                tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
                  NativeMessagePresentation.channel,
                  null,
                ),
          );
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

    testWidgets('collapses fixed sections in a short keyboard viewport', (
      tester,
    ) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(800, 220);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.view.resetPhysicalSize);
      const keyboardInset = 100.0;
      final prefs = await _mockPrefs();
      final harness = await _pumpMessageActionsPopover(
        tester,
        message: _message(),
        prefs: prefs,
        allMessages: [_message()],
        reminderService: _stubReminderService(),
        viewInsets: const EdgeInsets.only(bottom: keyboardInset),
      );

      expect(
        find.byKey(const ValueKey('message-action-reaction-tray')),
        findsNothing,
      );
      expect(
        find.byKey(const ValueKey('message-action-preview')),
        findsNothing,
      );
      expect(harness.sourceHidden.value, isFalse);
      final actionRect = tester.getRect(
        find.byKey(const ValueKey('message-action-surface')),
      );
      expect(actionRect.top, greaterThanOrEqualTo(Grid.xxs));
      expect(
        actionRect.bottom,
        lessThanOrEqualTo(220 - keyboardInset - Grid.xxs),
      );
      expect(tester.takeException(), isNull);

      await _dismissMessageActionsPopover(tester);
    });

    testWidgets('keeps tall message previews within the visible viewport', (
      tester,
    ) async {
      const keyboardInset = 300.0;
      final prefs = await _mockPrefs();
      await _pumpMessageActionsPopover(
        tester,
        message: _message(pubkey: 'self'),
        prefs: prefs,
        canManageMessage: true,
        allMessages: [_message(pubkey: 'self')],
        reminderService: _stubReminderService(),
        viewInsets: const EdgeInsets.only(bottom: keyboardInset),
        anchorRect: const Rect.fromLTWH(32, 40, 300, 2000),
      );

      final logicalHeight =
          tester.view.physicalSize.height / tester.view.devicePixelRatio;
      final visibleBottom = logicalHeight - keyboardInset - Grid.xxs;
      expect(
        tester
            .getRect(find.byKey(const ValueKey('message-action-preview')))
            .top,
        greaterThanOrEqualTo(Grid.xxs),
      );
      expect(
        tester
            .getRect(find.byKey(const ValueKey('message-action-surface')))
            .bottom,
        lessThanOrEqualTo(visibleBottom),
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

    testWidgets('does not restore composer focus after opening reactions', (
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

      await tester.tap(find.byKey(const ValueKey('quick-reaction-more')));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 400));
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

    testWidgets('ignores repeat action taps once dismissal starts', (
      tester,
    ) async {
      final prefs = await _mockPrefs();
      final harness = await _pumpMessageActionsPopover(
        tester,
        message: _message(rootId: 'root-9'),
        prefs: prefs,
        launcherOnNestedRoute: true,
      );
      final action = find.byKey(const ValueKey('message-action-followThread'));
      final actionWidget = tester.widget<InkWell>(action);

      actionWidget.onTap!.call();
      actionWidget.onTap!.call();
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('message-actions-underlying-page')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('message-actions-root-page')),
        findsNothing,
      );
      expect(harness.container.read(threadFollowsProvider).followedRootIds, {
        'root-9',
      });
      expect(tester.takeException(), isNull);
    });

    testWidgets('ignores repeat backdrop taps once dismissal starts', (
      tester,
    ) async {
      final prefs = await _mockPrefs();
      await _pumpMessageActionsPopover(
        tester,
        message: _message(),
        prefs: prefs,
        launcherOnNestedRoute: true,
      );
      final backdrop = tester.widget<GestureDetector>(
        find.byKey(const ValueKey('message-actions-backdrop')),
      );

      backdrop.onTap!.call();
      backdrop.onTap!.call();
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('message-actions-underlying-page')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('message-actions-root-page')),
        findsNothing,
      );
      expect(tester.takeException(), isNull);
    });

    testWidgets('ignores repeat quick reactions once dismissal starts', (
      tester,
    ) async {
      final prefs = await _mockPrefs();
      late _FakeChannelActions actions;
      await _pumpMessageActionsPopover(
        tester,
        message: _message(),
        prefs: prefs,
        launcherOnNestedRoute: true,
        createChannelActions: (ref) => actions = _FakeChannelActions(ref),
      );
      final reaction = find.byKey(const ValueKey('quick-reaction-\u{1F44D}'));
      final detector = tester.widget<GestureDetector>(
        find.descendant(of: reaction, matching: find.byType(GestureDetector)),
      );

      detector.onTap!.call();
      detector.onTap!.call();
      await tester.pumpAndSettle();

      expect(
        find.byKey(const ValueKey('message-actions-underlying-page')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('message-actions-root-page')),
        findsNothing,
      );
      expect(actions.reactions, [(eventId: 'msg-1', emoji: '\u{1F44D}')]);
      expect(tester.takeException(), isNull);
    });

    testWidgets('fallback action rows grow with accessibility text', (
      tester,
    ) async {
      final prefs = await _mockPrefs();
      await _pumpMessageActionsPopover(
        tester,
        message: _message(rootId: 'root-9'),
        prefs: prefs,
        textScaler: const TextScaler.linear(3),
      );

      final rowFinder = find.byKey(
        const ValueKey('message-action-followThread'),
      );
      expect(tester.getSize(rowFinder).height, greaterThan(48));
      expect(tester.takeException(), isNull);

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

    testWidgets('keeps the snapshot alive through the reverse transition', (
      tester,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        NativeMessagePresentation.channel,
        (_) async => null,
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          NativeMessagePresentation.channel,
          null,
        ),
      );
      ui.Image? snapshot;
      try {
        final prefs = await _mockPrefs();
        await _pumpMessageActionsPopover(
          tester,
          message: _message(),
          prefs: prefs,
          captureAnchorSnapshot: () async {
            snapshot = await _testMessageSnapshot();
            return snapshot!;
          },
        );

        final image = snapshot!;
        expect(image.debugDisposed, isFalse);
        Navigator.of(
          tester.element(find.byKey(const ValueKey('message-action-surface'))),
        ).pop();
        await tester.pump();

        expect(
          find.byKey(const ValueKey('message-action-preview')),
          findsOneWidget,
        );
        expect(image.debugDisposed, isFalse);
        await tester.pump(const Duration(milliseconds: 110));
        expect(image.debugDisposed, isFalse);

        await tester.pumpAndSettle();
        expect(image.debugDisposed, isTrue);
        expect(tester.takeException(), isNull);
      } finally {
        final image = snapshot;
        if (image != null && !image.debugDisposed) image.dispose();
        debugDefaultTargetPlatformOverride = null;
      }
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

      // Close the popover so its presentation guard does not block later
      // tests.
      Navigator.of(
        tester.element(find.byKey(const ValueKey('reaction-popover-tray'))),
      ).pop();
      await tester.pumpAndSettle();
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

    testWidgets('the popover uses channel catch-up without a profile', (
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
      expect(find.byType(AppListCard), findsNWidgets(2));
      expect(
        find.descendant(
          of: find.byType(SheetActionSection),
          matching: find.byType(ListTile),
        ),
        findsNWidgets(4),
      );
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
