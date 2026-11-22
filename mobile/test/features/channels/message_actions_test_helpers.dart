part of 'message_actions_test.dart';

TimelineMessage _message({
  String id = 'msg-1',
  String pubkey = 'alice',
  int createdAt = 1000,
  bool isSystem = false,
  String? rootId,
  List<List<String>> tags = const [],
  List<TimelineReaction> reactions = const [],
}) => TimelineMessage(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  content: 'hello world',
  isSystem: isSystem,
  rootId: rootId,
  tags: tags,
  reactions: reactions,
);

class _RecordingReactionActions implements ChannelActions {
  final added = <String>[];
  final targetAuthors = <String>[];
  final removed = <String>[];
  @override
  Future<void> addReaction(
    String eventId,
    String emoji, {
    required String targetAuthor,
  }) async {
    added.add(emoji);
    targetAuthors.add(targetAuthor);
  }

  @override
  Future<void> removeReaction(String eventId, String emoji) async =>
      removed.add(eventId);
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

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
    bool explicit = false,
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
  bool markContextUnread(
    String contextId, {
    required String channelId,
    String? threadRootId,
    bool channelCatchUp = false,
  }) {
    markedUnread.add(contextId);
    state = _withForced({...state.forcedUnreadContexts, contextId: channelId});
    return true;
  }

  @override
  Future<bool> setThreadFollow(
    String channelId,
    String rootId,
    bool follow,
  ) async {
    state = ReadStateState(
      isReady: state.isReady,
      pubkey: state.pubkey,
      contexts: state.contexts,
      version: state.version + 1,
      forcedUnreadContexts: state.forcedUnreadContexts,
      threadFollows: {
        ...state.threadFollows,
        threadFollowKey(channelId, rootId): (follow: follow, at: 1),
      },
      threadFollowsIncomplete: false,
    );
    return true;
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
  http.Client? mediaClient,
}) async {
  await tester.pumpWidget(
    ProviderScope(
      overrides: [
        if (mediaClient != null)
          mediaHttpClientProvider.overrideWithValue(mediaClient),
      ],
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
  EmojiDataset? emojiDataset,
  EdgeInsets viewInsets = EdgeInsets.zero,
  TextScaler textScaler = TextScaler.noScaling,
  Future<ui.Image> Function()? captureAnchorSnapshot,
  FocusNode? composerFocusNode,
  bool composerInitiallyFocused = false,
  String? composerDraft,
  bool launcherOnNestedRoute = false,
  ChannelActions Function(Ref ref)? createChannelActions,
  Rect anchorRect = const Rect.fromLTWH(32, 260, 300, 72),
  String? currentPubkey = 'self',
  bool listChannel = false,
}) async {
  final sourceHidden = ValueNotifier(false);
  final draftController = composerDraft == null
      ? null
      : TextEditingController(text: composerDraft);
  if (draftController != null) addTearDown(draftController.dispose);

  Widget launcherPage() => Scaffold(
    key: const ValueKey('message-actions-underlying-page'),
    body: Consumer(
      builder: (context, ref, _) {
        // The channel page keeps the channel list alive.
        if (listChannel) ref.watch(channelsProvider);
        return Column(
          children: [
            if (composerFocusNode != null)
              TextField(
                focusNode: composerFocusNode,
                controller: draftController,
              ),
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
        if (emojiDataset != null)
          emojiDatasetProvider.overrideWith((_) async => emojiDataset),
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
