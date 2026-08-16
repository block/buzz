import 'package:buzz/features/channels/message_actions.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/features/channels/thread_follows/thread_follows_provider.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/reminders/reminder_service.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
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

void main() {
  group('showMessageActions', () {
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
