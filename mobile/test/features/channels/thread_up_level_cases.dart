part of 'channel_detail_page_test.dart';

/// A relay that answers exact-id lookups once [gate] opens, or fails them.
class _ParentLookupSession extends RelaySessionNotifier {
  _ParentLookupSession(this.events, {this.fail = false});

  final List<NostrEvent> events;
  final bool fail;
  final gate = Completer<void>();
  final requested = <String>[];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final ids = filter.ids ?? const <String>[];
    if (ids.isEmpty) return [];
    requested.addAll(ids);
    await gate.future;
    if (fail) throw StateError('offline');
    return [
      for (final event in events)
        if (ids.contains(event.id)) event,
    ];
  }
}

/// Records which pinned messages the pages let go of.
class _PinRecordingMessagesNotifier extends _FakeMessagesNotifier {
  _PinRecordingMessagesNotifier(super.messages);

  final released = <String>{};

  @override
  void releaseDeepLinkEvents(Iterable<String> eventIds) {
    released.addAll(eventIds);
    super.releaseDeepLinkEvents(eventIds);
  }
}

final _threadUpRow = find.byKey(const ValueKey('thread-up-level'));

final _threadUpChevron = find.byKey(const ValueKey('thread-up-level-chevron'));
final _threadUpLoading = find.byKey(const ValueKey('thread-up-level-loading'));

String _threadUpLabel(WidgetTester tester) => tester
    .widget<Text>(find.byKey(const ValueKey('thread-up-level-label')))
    .data!;

/// The glyph leading the label, or null when it has none.
Icon? _threadUpIcon(WidgetTester tester) => tester
    .widgetList<Icon>(find.byKey(const ValueKey('thread-up-level-icon')))
    .firstOrNull;

ThreadDetailPage _shownThread(WidgetTester tester) =>
    tester.widget<ThreadDetailPage>(find.byType(ThreadDetailPage));

Finder _channelRow(String messageId, {bool skipOffstage = true}) =>
    find.byKey(ValueKey('message-row-$messageId'), skipOffstage: skipOffstage);

/// Whether the channel shows [messageId] between its header and composer.
bool _channelShows(WidgetTester tester, String messageId) {
  final row = _channelRow(messageId);
  if (row.evaluate().isEmpty) return false;
  final rect = tester.getRect(row);
  final list = tester.getRect(
    find.byKey(const ValueKey('channel-message-list')),
  );
  return rect.bottom > list.top + 100 && rect.top < list.bottom - 100;
}

/// A top-level thread with enough later channel messages that its root is
/// out of view when the channel opens at its newest message.
({NostrEvent root, NostrEvent target, List<NostrEvent> channel})
_threadBelowTheFold() {
  final root = _textMsg(
    id: threadIdRoot,
    pubkey: 'alice',
    content: 'Thread root',
    createdAt: 1000,
  );
  final target = _textMsg(
    id: 'target',
    pubkey: 'bob',
    content: 'Target reply',
    createdAt: 90000,
    extraTags: const [
      ['e', threadIdRoot, '', 'reply'],
    ],
  );
  return (
    root: root,
    target: target,
    channel: [
      root,
      for (var index = 0; index < 40; index++)
        _textMsg(
          id: 'later-$index',
          pubkey: index.isEven ? 'bob' : 'carol',
          content: 'Later message $index',
          createdAt: 2000 + index * 1000,
        ),
      target,
    ],
  );
}

const _threadUpUsers = {
  'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
  'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
  'carol': UserProfile(pubkey: 'carol', displayName: 'Carol'),
};

Channel _threadUpChannel({
  required String name,
  String channelType = 'stream',
  String visibility = 'open',
  List<String> labels = const [],
  List<String> participants = const [],
  List<String> participantPubkeys = const [],
}) => Channel(
  id: _channelId,
  name: name,
  channelType: channelType,
  labels: labels,
  visibility: visibility,
  description: '',
  createdBy: 'abc123',
  createdAt: DateTime(2025),
  memberCount: 2,
  participants: participants,
  participantPubkeys: participantPubkeys,
  isMember: true,
);

/// How the thread under test reached the screen.
enum _ThreadLanding {
  /// Pushed over the channel page that opened it.
  pushed,

  /// Pushed over the channel page inside a conversation pane.
  pane,

  /// In place of the channel page that opened it, over the Inbox.
  replaced,

  /// In place of the channel page that opened it, over the same channel.
  replacedOverChannel,
}

void threadUpLevelTests() {
  group('thread up one level', () {
    for (final (kind, channel, legacySolo, icon, label, dark, textScale)
        in <(String, Channel, bool, IconData?, String, bool, double)>[
          (
            'public channel',
            _testChannel,
            false,
            BuzzIcons.hash,
            'general',
            false,
            1,
          ),
          (
            'private channel',
            _threadUpChannel(name: 'leads', visibility: 'private'),
            false,
            BuzzIcons.lock,
            'leads',
            true,
            1,
          ),
          (
            'DM',
            _threadUpChannel(
              name: 'DM',
              channelType: 'dm',
              participantPubkeys: ['self', 'alice'],
              participants: ['Me', 'Alice'],
            ),
            false,
            // The Messages list leads a DM with avatars, not a glyph.
            null,
            'DM · Alice',
            false,
            1,
          ),
          (
            'Solo chat in large text',
            _threadUpChannel(name: 'Research notes', labels: ['buzz.solo']),
            false,
            BuzzIcons.messageCircle,
            'Research notes',
            true,
            2,
          ),
          (
            'Solo chat from before the Solo label',
            _threadUpChannel(name: 'Old notes', visibility: 'private'),
            true,
            BuzzIcons.messageCircle,
            'Old notes',
            false,
            1,
          ),
        ]) {
      testWidgets('names its $kind as the Messages list marks it, and is '
          'plain when that is unresolved', (tester) async {
        final semantics = tester.ensureSemantics();
        final channels = _FakeChannelsNotifier([channel]);
        final root = _textMsg(id: threadIdRoot, pubkey: 'alice', content: 'Hi');
        final timeline = formatTimeline([root]);
        final legacy = legacySolo ? [channel.id] : const <String>[];
        await tester.pumpWidget(
          _buildTestable(
            messages: [root],
            channel: channel,
            channelsNotifier: channels,
            users: _threadUpUsers,
            darkTheme: dark,
            textScaler: TextScaler.linear(textScale),
            organization: Stream.value(MeGroups(channels: legacy)),
            threadReplies: const {threadIdRoot: []},
            home: ThreadDetailPage(
              threadHead: timeline.single,
              allMessages: timeline,
              channelId: channel.id,
              currentPubkey: 'self',
              isMember: true,
              isArchived: false,
            ),
          ),
        );
        await tester.pumpAndSettle();

        // The list's glyph stands where a "#" would otherwise be written.
        expect(_threadUpLabel(tester), label);
        expect(_threadUpIcon(tester)?.icon, icon);
        expect(icon, conversationListIcon(channel, legacySoloChannels: legacy));
        // It reads as "opens something", in the theme's own muted ink.
        final theme = Theme.of(tester.element(_threadUpRow));
        expect(theme.brightness, dark ? Brightness.dark : Brightness.light);
        final muted = theme.colorScheme.onSurfaceVariant;
        final chevron = tester.widget<Icon>(_threadUpChevron);
        expect(chevron.icon, BuzzIcons.chevronRight);
        expect(chevron.color, muted);
        expect(_threadUpIcon(tester)?.color, icon == null ? null : muted);
        expect(_threadUpLoading, findsNothing);
        // One stop that says where it goes; the visible text is not a second.
        expect(
          tester.getSemantics(_threadUpRow),
          matchesSemantics(
            label: 'Go to $label',
            isButton: true,
            hasEnabledState: true,
            isEnabled: true,
            hasTapAction: true,
          ),
        );
        expect(find.bySemanticsLabel(label), findsNothing);
        expect(find.bySemanticsLabel('Thread'), findsNothing);
        // Large text grows the bar instead of clipping the title, and the
        // thread still starts below it.
        final head = find.byKey(ValueKey('thread-message-group-$threadIdRoot'));
        final headTop = tester.getRect(head).top;
        expect(
          headTop,
          greaterThanOrEqualTo(tester.getRect(_threadUpRow).bottom),
        );

        // Unresolved, the title is the plain one it was before it led up,
        // and at the default text size leading up took no height from the
        // thread.
        channels.setChannels([]);
        await tester.pumpAndSettle();
        expect(_threadUpRow, findsNothing);
        expect(
          find.byKey(const ValueKey('thread-up-level-label')),
          findsNothing,
        );
        expect(
          tester
              .widget<Text>(find.byKey(const ValueKey('thread-app-bar-title')))
              .data,
          'Thread',
        );
        expect(find.bySemanticsLabel(RegExp('^Go to ')), findsNothing);
        if (textScale == 1) expect(tester.getRect(head).top, headTop);
        expect(tester.takeException(), isNull);
        semantics.dispose();
      });
    }

    for (final (entry, landing, size) in [
      ('a push notification', _ThreadLanding.pushed, null),
      ('the Inbox on a phone', _ThreadLanding.pane, Size(400, 800)),
      ('the Inbox pane', _ThreadLanding.pane, Size(1100, 800)),
      ('an Inbox without a pane', _ThreadLanding.replaced, null),
      (
        'a notification over its open channel',
        _ThreadLanding.replacedOverChannel,
        null,
      ),
    ]) {
      testWidgets('goes from a thread opened by $entry to its conversation at '
          'the root message, without a second channel page', (tester) async {
        if (size != null) {
          tester.view.devicePixelRatio = 1;
          tester.view.physicalSize = size;
          addTearDown(tester.view.reset);
        }
        final thread = _threadBelowTheFold();
        final notifier = _FakeMessagesNotifier(thread.channel);
        ChannelDetailPage landingPage(InitialThreadRouteBehavior behavior) =>
            ChannelDetailPage(
              channel: _testChannel,
              initialMessageId: 'target',
              initialThreadRootId: threadIdRoot,
              initialThreadRouteBehavior: behavior,
            );
        // The Inbox opens its row exactly these two ways.
        Widget inbox(void Function(BuildContext context) open) => Builder(
          builder: (context) => Scaffold(
            body: Center(
              child: TextButton(
                onPressed: () => open(context),
                child: const Text('Open inbox row'),
              ),
            ),
          ),
        );
        final replaces = MaterialPageRoute<void>(
          builder: (_) =>
              landingPage(InitialThreadRouteBehavior.replaceCurrentRoute),
        );
        await tester.pumpWidget(
          _buildTestable(
            messages: thread.channel,
            messagesNotifier: notifier,
            initialMessageId: landing == _ThreadLanding.pushed
                ? 'target'
                : null,
            initialThreadRootId: landing == _ThreadLanding.pushed
                ? threadIdRoot
                : null,
            threadReplies: {
              threadIdRoot: [thread.target],
            },
            users: _threadUpUsers,
            navigatorObservers: [appRouteObserver],
            home: switch (landing) {
              _ThreadLanding.pane => AdaptiveConversationDestination(
                child: inbox(
                  (context) => openConversationPane(
                    context,
                    'target',
                    (_) => landingPage(InitialThreadRouteBehavior.push),
                    reopen: true,
                  ),
                ),
              ),
              _ThreadLanding.replaced => inbox(
                (context) => Navigator.of(context).push(replaces),
              ),
              _ => null,
            },
          ),
        );
        await tester.pumpAndSettle();
        switch (landing) {
          case _ThreadLanding.pushed:
            break;
          case _ThreadLanding.pane || _ThreadLanding.replaced:
            await tester.tap(find.text('Open inbox row'));
          case _ThreadLanding.replacedOverChannel:
            unawaited(
              tester
                  .state<NavigatorState>(find.byType(Navigator))
                  .push(replaces),
            );
        }
        await tester.pumpAndSettle();

        expect(_shownThread(tester).threadHead.id, threadIdRoot);
        expect(_threadUpLabel(tester), 'general');
        // No channel beneath has jumped to the root: it was never laid out.
        expect(_channelRow(threadIdRoot, skipOffstage: false), findsNothing);

        // A second tap before the first has left changes nothing.
        await tester.tap(_threadUpRow);
        await tester.tap(_threadUpRow, warnIfMissed: false);
        await tester.pumpAndSettle();

        // With no channel beneath, the thread stays for Back to return to.
        expect(
          find.byType(ThreadDetailPage, skipOffstage: false),
          landing == _ThreadLanding.replaced ? findsOneWidget : findsNothing,
        );
        expect(
          find.byType(ChannelDetailPage, skipOffstage: false),
          findsOneWidget,
        );
        expect(_channelShows(tester, threadIdRoot), isTrue);
        if (landing == _ThreadLanding.pane) {
          // The pane's own navigator moved; the Inbox is still the root.
          expect(
            find.text('Open inbox row', skipOffstage: false),
            findsOneWidget,
          );
          expect(
            Navigator.of(
              tester.element(find.byType(AdaptiveConversationDestination)),
            ).canPop(),
            isFalse,
          );
        }

        // The channel stays there when the next message arrives.
        notifier.setMessages([
          ...thread.channel,
          _textMsg(
            id: 'live',
            pubkey: 'carol',
            content: 'Just arrived',
            createdAt: 91000,
          ),
        ]);
        await tester.pumpAndSettle();
        expect(_channelShows(tester, threadIdRoot), isTrue);

        // Reopened from its own row, the thread goes back to the channel
        // where the reader left it, and a keyboard reaches the row too.
        await tester.drag(
          find.byKey(const ValueKey('channel-message-list')),
          const Offset(0, -24),
        );
        await tester.pumpAndSettle();
        final rootTop = tester.getRect(_channelRow(threadIdRoot)).top;
        await tester.tap(find.text('1 reply'));
        await tester.pumpAndSettle();
        expect(_shownThread(tester).threadHead.id, threadIdRoot);
        Focus.of(
          tester.element(find.byKey(const ValueKey('thread-up-level-label'))),
        ).requestFocus();
        await tester.pump();
        await tester.sendKeyEvent(LogicalKeyboardKey.enter);
        await tester.pumpAndSettle();
        expect(find.byType(ThreadDetailPage), findsNothing);
        expect(
          find.byType(ChannelDetailPage, skipOffstage: false),
          findsOneWidget,
        );
        expect(tester.getRect(_channelRow(threadIdRoot)).top, rootTop);
        expect(tester.takeException(), isNull);
      });
    }

    testWidgets('finds the root again after the channel beneath has moved '
        'its window past it', (tester) async {
      final thread = _threadBelowTheFold();
      final notifier = _PinRecordingMessagesNotifier([
        thread.root,
        thread.target,
      ]);
      final session = _ParentLookupSession([thread.root])..gate.complete();
      await tester.pumpWidget(
        _buildTestable(
          messages: [thread.root, thread.target],
          messagesNotifier: notifier,
          relaySessionNotifier: session,
          threadReplies: {
            threadIdRoot: [thread.target],
          },
          users: _threadUpUsers,
          navigatorObservers: [appRouteObserver],
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('1 reply'));
      await tester.pumpAndSettle();
      // A reconnect reloads the newest page, which no longer reaches the root.
      notifier.setMessages(thread.channel.sublist(1));
      await tester.pumpAndSettle();

      await tester.tap(_threadUpRow);
      await tester.pumpAndSettle();

      expect(find.byType(ThreadDetailPage), findsNothing);
      expect(session.requested, contains(threadIdRoot));
      expect(_channelShows(tester, threadIdRoot), isTrue);
      expect(notifier.released, isEmpty);

      // Leaving the channel stops holding the root in its window.
      await tester.pumpWidget(const SizedBox());
      expect(notifier.released, {threadIdRoot});
      expect(tester.takeException(), isNull);
    });

    testWidgets('walks from a sub-thread to its parent thread, then to the '
        'conversation, and Back retraces it', (tester) async {
      final semantics = tester.ensureSemantics();
      final thread = _threadBelowTheFold();
      final parent = _textMsg(
        id: threadIdParent,
        pubkey: 'bob',
        content: 'Parent reply',
        createdAt: 1100,
        extraTags: const [
          ['e', threadIdRoot, '', 'reply'],
        ],
      );
      final target = _textMsg(
        id: 'nested-target',
        pubkey: 'carol',
        content: 'Deeply nested target',
        createdAt: 1200,
        extraTags: const [
          ['e', threadIdRoot, '', 'root'],
          ['e', threadIdParent, '', 'reply'],
        ],
      );
      await tester.pumpWidget(
        _buildTestable(
          messages: [...thread.channel, parent, target],
          initialMessageId: 'nested-target',
          initialThreadRootId: threadIdParent,
          threadReplies: {
            threadIdRoot: [parent, target],
          },
          users: _threadUpUsers,
          navigatorObservers: [appRouteObserver],
        ),
      );
      await tester.pumpAndSettle();
      // Let the landing's own highlight finish first.
      await tester.pump(const Duration(seconds: 4));

      expect(_shownThread(tester).threadHead.id, threadIdParent);
      // A sub-thread names its parent thread, marked as a thread.
      expect(_threadUpLabel(tester), 'Thread root');
      expect(_threadUpIcon(tester)?.icon, BuzzIcons.messageSquareReply);
      expect(_threadUpChevron, findsOneWidget);
      expect(
        tester.getSemantics(_threadUpRow),
        matchesSemantics(
          label: 'Go to parent thread',
          isButton: true,
          hasEnabledState: true,
          isEnabled: true,
          hasTapAction: true,
        ),
      );

      await tester.tap(_threadUpRow);
      await tester.pumpAndSettle();

      // The parent thread, with the sub-thread's head revealed and marked.
      expect(_shownThread(tester).threadHead.id, threadIdRoot);
      expect(_shownThread(tester).initialMessageId, threadIdParent);
      final highlight =
          tester
                  .widget<DecoratedBox>(
                    find.byKey(
                      const ValueKey('thread-message-$threadIdParent'),
                    ),
                  )
                  .decoration
              as BoxDecoration;
      expect(highlight.color!.a, closeTo(0.12, 0.001));
      expect(_threadUpLabel(tester), 'general');
      expect(_threadUpIcon(tester)?.icon, BuzzIcons.hash);

      // A screen reader's activation takes the second step, once: a repeat
      // before the page has changed opens nothing more.
      tester.semantics.tap(find.semantics.byLabel('Go to general'));
      tester.semantics.tap(find.semantics.byLabel('Go to general'));
      await tester.pumpAndSettle();

      expect(find.byType(ThreadDetailPage), findsNothing);
      expect(_channelShows(tester, threadIdRoot), isTrue);

      // Each step was pushed over the thread it left.
      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      expect(_shownThread(tester).threadHead.id, threadIdRoot);
      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      expect(_shownThread(tester).threadHead.id, threadIdParent);

      // Opened from its row in the parent thread, a sub-thread goes back to
      // that page rather than stacking a copy of it.
      await tester.tap(_threadUpRow);
      await tester.pumpAndSettle();
      await tester.tap(
        find.byKey(const ValueKey('nested-thread-summary-$threadIdParent')),
      );
      await tester.pumpAndSettle();
      expect(_shownThread(tester).threadHead.id, threadIdParent);
      expect(
        find.byType(ThreadDetailPage, skipOffstage: false),
        findsNWidgets(3),
      );
      await tester.tap(_threadUpRow);
      await tester.pumpAndSettle();
      expect(_shownThread(tester).threadHead.id, threadIdRoot);
      expect(
        find.byType(ThreadDetailPage, skipOffstage: false),
        findsNWidgets(2),
      );
      expect(tester.takeException(), isNull);
      semantics.dispose();
    });

    for (final fail in [false, true]) {
      testWidgets('waits, inert, for a parent message that is not loaded, '
          'then ${fail ? 'falls back to the conversation' : 'opens it'}', (
        tester,
      ) async {
        final semantics = tester.ensureSemantics();
        final root = _textMsg(
          id: threadIdRoot,
          pubkey: 'alice',
          content:
              'See [the *plan*](https://example.org/plan)\nfor `details` '
              '${'x' * 100}',
          createdAt: 1000,
        );
        final head = _textMsg(
          id: threadIdParent,
          pubkey: 'bob',
          content: 'Sub-thread head',
          createdAt: 1100,
          extraTags: const [
            ['e', threadIdRoot, '', 'reply'],
          ],
        );
        final session = _ParentLookupSession([root], fail: fail);
        final notifier = _PinRecordingMessagesNotifier(const []);
        final timeline = formatTimeline([head]);
        await tester.pumpWidget(
          _buildTestable(
            // The root is older than the channel's loaded window.
            messages: const [],
            messagesNotifier: notifier,
            relaySessionNotifier: session,
            threadReplies: {
              threadIdRoot: [head],
            },
            users: _threadUpUsers,
            navigatorObservers: [appRouteObserver],
            home: ThreadDetailPage(
              threadHead: timeline.single,
              allMessages: timeline,
              channelId: _channelId,
              currentPubkey: 'self',
              isMember: true,
              isArchived: false,
            ),
          ),
        );
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 100));

        expect(session.requested, [threadIdRoot]);
        // It shows that it is loading rather than offering a tap that
        // would do nothing.
        expect(_threadUpLabel(tester), 'Parent thread');
        expect(_threadUpIcon(tester)?.icon, BuzzIcons.messageSquareReply);
        expect(_threadUpLoading, findsOneWidget);
        expect(_threadUpChevron, findsNothing);
        expect(
          tester.getSemantics(_threadUpRow),
          matchesSemantics(
            label: 'Loading parent thread',
            hasEnabledState: true,
          ),
        );
        await tester.tap(_threadUpRow);
        await tester.pump(const Duration(milliseconds: 100));
        expect(_shownThread(tester).threadHead.id, threadIdParent);

        session.gate.complete();
        await tester.pumpAndSettle();
        expect(session.requested, [threadIdRoot]);
        expect(
          _threadUpLabel(tester),
          fail ? 'general' : 'See the plan for details ${'x' * 55}',
        );
        expect(
          _threadUpIcon(tester)?.icon,
          fail ? BuzzIcons.hash : BuzzIcons.messageSquareReply,
        );
        expect(_threadUpLoading, findsNothing);
        // A long snippet ends in an ellipsis inside the bar, chevron and all.
        final barRight =
            tester.view.physicalSize.width / tester.view.devicePixelRatio -
            Grid.gutter;
        expect(tester.getRect(_threadUpRow).right, lessThanOrEqualTo(barRight));
        expect(
          tester.getRect(_threadUpChevron).right,
          lessThanOrEqualTo(barRight),
        );
        expect(
          tester
              .renderObject<RenderParagraph>(
                find.descendant(
                  of: find.byKey(const ValueKey('thread-up-level-label')),
                  matching: find.byType(RichText),
                ),
              )
              .didExceedMaxLines,
          !fail,
        );

        await tester.tap(_threadUpRow);
        await tester.pumpAndSettle();
        if (fail) {
          expect(find.byType(ThreadDetailPage), findsNothing);
          expect(find.byType(ChannelDetailPage), findsOneWidget);
        } else {
          expect(_shownThread(tester).threadHead.id, threadIdRoot);
          expect(_shownThread(tester).initialMessageId, threadIdParent);
        }

        // The lookup's hold on the parent ends, at the latest, with the page.
        semantics.dispose();
        await tester.pumpWidget(const SizedBox());
        expect(notifier.released, contains(threadIdRoot));
        expect(tester.takeException(), isNull);
      });
    }

    testWidgets('does not fetch a parent message that the thread\'s own '
        'replies bring', (tester) async {
      final parent = _textMsg(
        id: threadIdParent,
        pubkey: 'bob',
        content: 'Parent reply',
        createdAt: 1100,
        extraTags: const [
          ['e', threadIdRoot, '', 'reply'],
        ],
      );
      final head = _textMsg(
        id: 'nested-head',
        pubkey: 'carol',
        content: 'Sub-thread head',
        createdAt: 1200,
        extraTags: const [
          ['e', threadIdRoot, '', 'root'],
          ['e', threadIdParent, '', 'reply'],
        ],
      );
      final session = _ParentLookupSession(const [])..gate.complete();
      final timeline = formatTimeline([head]);
      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          relaySessionNotifier: session,
          threadReplies: {
            threadIdRoot: [parent, head],
          },
          users: _threadUpUsers,
          home: ThreadDetailPage(
            threadHead: timeline.single,
            allMessages: timeline,
            channelId: _channelId,
            currentPubkey: 'self',
            isMember: true,
            isArchived: false,
          ),
        ),
      );
      await tester.pumpAndSettle();

      expect(_threadUpLabel(tester), 'Parent reply');
      expect(session.requested, isEmpty);
      expect(tester.takeException(), isNull);
    });
  });
}
