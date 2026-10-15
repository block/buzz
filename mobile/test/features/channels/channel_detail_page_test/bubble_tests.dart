part of '../channel_detail_page_test.dart';

void bubbleLayoutTests() {
  testWidgets('large activity members build and fetch only visible rows', (
    tester,
  ) async {
    final loaded = <String>{};
    final cache = _FakeUserCacheNotifier(
      {},
      preload: (pubkeys) async {
        loaded.addAll(pubkeys);
        return true;
      },
    );
    await tester.pumpWidget(
      _buildTestable(
        messages: [
          for (var index = 0; index < 1000; index++)
            _systemMsg(
              id: 'bulk-${index.toString().padLeft(4, '0')}',
              payload: {
                'type': 'member_removed',
                'actor': 'alice',
                'target': 'target-$index',
              },
            ),
        ],
        userCacheNotifier: cache,
      ),
    );
    await tester.pumpAndSettle();
    loaded.clear();
    await tester.tap(find.text('See all'));
    await tester.pumpAndSettle();
    expect(
      find.byKey(const ValueKey('activity-member-target-999')),
      findsNothing,
    );
    expect(loaded.length, allOf(greaterThan(0), lessThan(20)));
    final list = find.byKey(const ValueKey('activity-members-list'));
    final scrollable = find.descendant(
      of: list,
      matching: find.byType(Scrollable),
    );
    await tester.scrollUntilVisible(
      find.byKey(const ValueKey('activity-member-target-50')),
      300,
      scrollable: scrollable,
    );
    await tester.pumpAndSettle();
    expect(loaded, contains('target-50'));
    expect(loaded.length, lessThan(100));
    final beforeUpdate = {...loaded};
    cache.replace(
      const UserProfile(pubkey: 'target-50', displayName: 'Updated target'),
    );
    await tester.pumpAndSettle();
    expect(find.text('Updated target'), findsOneWidget);
    expect(loaded, beforeUpdate);
    expect(tester.takeException(), isNull);
  });

  testWidgets(
    'status bar pagination is capped and later taps continue',
    (tester) async {
      final notifier = _FakeMessagesNotifier(
        [
          for (var index = 0; index < 30; index++)
            _textMsg(
              id: 'recent-$index',
              pubkey: 'alice',
              content: 'Recent $index',
              createdAt: 3000 + index,
            ),
        ],
        olderPages: [
          for (var index = 1; index <= 10; index++)
            [
              _textMsg(
                id: 'page-$index',
                pubkey: 'alice',
                content: 'Older page $index',
                createdAt: 3000 - index * 100,
              ),
            ],
        ],
      );
      await tester.pumpWidget(
        _buildTestable(messages: const [], messagesNotifier: notifier),
      );
      await tester.pumpAndSettle();
      expect(notifier.fetchOlderCalls, 0);
      for (final expected in [3, 6, 9, 10]) {
        await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
          'flutter/status_bar',
          const JSONMethodCodec().encodeMethodCall(
            const MethodCall('handleScrollToTop'),
          ),
          (_) {},
        );
        await tester.pumpAndSettle();
        expect(notifier.fetchOlderCalls, expected);
        expect(find.text('Older page $expected').hitTestable(), findsOneWidget);
        await tester.pumpAndSettle();
        expect(
          notifier.fetchOlderCalls,
          expected,
          reason: 'landing must not bypass the gesture cap',
        );
      }
      expect(notifier.reachedOldest, isTrue);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

  testWidgets(
    'grouped removal quick reactions remove reactions on later events',
    (tester) async {
      final actions = _GroupedReactionActions();
      await tester.pumpWidget(
        _buildTestable(
          signingPubkey: 'self',
          messages: [
            for (var index = 0; index < 3; index++)
              _systemMsg(
                id: 'removal-$index',
                payload: {
                  'type': 'member_removed',
                  'actor': 'alice',
                  'target': 'target-$index',
                },
                createdAt: 1000 + index,
              ),
            for (var index = 1; index < 3; index++)
              _reaction(
                id: 'own-$index',
                targetId: 'removal-$index',
                pubkey: 'self',
              ),
          ],
          createChannelActions: (_) => actions,
        ),
      );
      await tester.pumpAndSettle();
      await tester.longPress(
        find.byKey(const ValueKey('system-message-row-removal-0')),
      );
      await tester.pumpAndSettle();
      final quick = find.byKey(const ValueKey('quick-reaction-👍'));
      expect(quick, findsOneWidget);
      expect(
        find.descendant(
          of: quick,
          matching: find.byWidgetPredicate(
            (widget) =>
                widget is Semantics && widget.properties.selected == true,
          ),
        ),
        findsOneWidget,
      );
      await tester.tap(quick);
      await tester.pumpAndSettle();
      expect(actions.removed, ['own-1', 'own-2']);
      expect(actions.added, isEmpty);
    },
  );

  testWidgets(
    'status bar navigation fetches short history and stops on failure',
    (tester) async {
      for (final fail in [false, true]) {
        final notifier = _FakeMessagesNotifier(
          [
            for (var index = 0; index < 30; index++)
              _textMsg(
                id: 'recent-$index',
                pubkey: 'alice',
                content: 'Recent $index',
                createdAt: 3000 + index,
              ),
          ],
          olderPages: [
            [
              _textMsg(
                id: 'middle',
                pubkey: 'alice',
                content: 'Middle page',
                createdAt: 2000,
              ),
            ],
            [
              _textMsg(
                id: 'oldest',
                pubkey: 'alice',
                content: 'First page',
                createdAt: 1000,
              ),
            ],
          ],
          failOlderFetch: fail,
        );
        await tester.pumpWidget(
          _buildTestable(messages: const [], messagesNotifier: notifier),
        );
        await tester.pumpAndSettle();
        expect(notifier.fetchOlderCalls, 0);
        await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
          'flutter/status_bar',
          const JSONMethodCodec().encodeMethodCall(
            const MethodCall('handleScrollToTop'),
          ),
          (_) {},
        );
        await tester.pumpAndSettle();
        expect(notifier.fetchOlderCalls, fail ? 1 : 2);
        if (!fail) {
          expect(find.text('First page').hitTestable(), findsOneWidget);
        }
        expect(tester.takeException(), isNull);
        await tester.pumpWidget(const SizedBox());
      }
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

  for (final thread in [false, true]) {
    for (final tagged in [false, true]) {
      testWidgets(
        'attachments isolate both group boundaries, thread=$thread tagged=$tagged',
        (tester) async {
          final events = [
            if (thread)
              _textMsg(
                id: threadIdRoot,
                pubkey: 'bob',
                content: 'Root',
                createdAt: 900,
              ),
            for (var index = 0; index < 3; index++)
              _textMsg(
                id: 'boundary-$index',
                pubkey: 'alice',
                createdAt: 1000 + index,
                content: index == 1
                    ? '![photo](https://example.com/photo.png)'
                    : 'Text $index',
                extraTags: [
                  if (thread) ['e', threadIdRoot, '', 'reply'],
                  if (tagged && index == 1)
                    [
                      'imeta',
                      'url https://example.com/photo.png',
                      'm image/png',
                    ],
                ],
              ),
          ];
          final messages = formatTimeline(events);
          await tester.pumpWidget(
            _buildTestable(
              messages: events,
              home: thread
                  ? ThreadDetailPage(
                      threadHead: messages.firstWhere(
                        (message) => message.id == threadIdRoot,
                      ),
                      allMessages: messages,
                      channelId: _channelId,
                      currentPubkey: 'me',
                      isMember: true,
                      isArchived: false,
                    )
                  : null,
            ),
          );
          await tester.pumpAndSettle();
          for (var index = 0; index < 3; index++) {
            if (thread) {
              final list = tester.widget<ScrollablePositionedList>(
                find.byKey(const ValueKey('thread-message-list')),
              );
              list.itemScrollController!.jumpTo(index: index + 1);
              await tester.pumpAndSettle();
            }
            final row = find.byKey(
              ValueKey(
                '${thread ? 'thread-message-row' : 'message-row'}-boundary-$index',
              ),
            );
            final content = tester.widget<MessageContent>(
              find.descendant(of: row, matching: find.byType(MessageContent)),
            );
            expect(content.bubbleStart, isTrue, reason: 'start $index');
            expect(content.bubbleEnd, isTrue, reason: 'end $index');
            expect(
              find.descendant(of: row, matching: find.byType(AvatarImage)),
              findsOneWidget,
            );
          }
        },
      );
    }
  }

  testWidgets(
    'legacy purpose activity is hidden without splitting the sender group',
    (tester) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(
              id: 'before-purpose',
              pubkey: 'alice',
              content: 'Before',
              createdAt: 1000,
            ),
            _systemMsg(
              id: 'purpose-change',
              payload: {
                'type': 'purpose_changed',
                'actor': 'alice',
                'purpose': 'Legacy purpose',
              },
              createdAt: 1010,
            ),
            _reaction(id: 'purpose-reaction', targetId: 'purpose-change'),
            _textMsg(
              id: 'after-purpose',
              pubkey: 'alice',
              content: 'After',
              createdAt: 1020,
            ),
            _systemMsg(
              id: 'topic-change',
              payload: {
                'type': 'topic_changed',
                'actor': 'alice',
                'topic': 'Current topic',
              },
              createdAt: 1030,
            ),
          ],
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
        ),
      );
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('system-message-row-purpose-change')),
        findsNothing,
      );
      expect(findRichText('Legacy purpose'), findsNothing);
      expect(find.byType(ReactionRow), findsNothing);
      expect(
        find.byKey(const ValueKey('message-author-before-purpose')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('message-author-after-purpose')),
        findsNothing,
      );
      expect(
        findRichText('changed the topic to "Current topic"'),
        findsOneWidget,
      );
    },
  );

  testWidgets('single-line activity uses text without an icon', (tester) async {
    await tester.pumpWidget(
      _buildTestable(
        messages: [
          _systemMsg(
            id: 'single-line',
            payload: {'type': 'member_left', 'actor': 'alice'},
          ),
        ],
        users: {
          'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
        },
      ),
    );
    await tester.pumpAndSettle();
    final card = find.byKey(const ValueKey('activity-card-single-line'));
    final icon = find.descendant(of: card, matching: find.byType(Icon));
    expect(icon, findsNothing);
    expect(findRichText('left the channel'), findsOneWidget);
  });

  testWidgets(
    'keeps centered activities timestamp-free at accessibility text sizes',
    (tester) async {
      tester.view.physicalSize = const Size(240, 600);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _systemMsg(
              id: 'sys-accessible',
              payload: {'type': 'member_left', 'actor': 'alice'},
              createdAt:
                  DateTime(2026, 7, 28, 12, 34).millisecondsSinceEpoch ~/ 1000,
            ),
          ],
          users: {
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
          textScaler: const TextScaler.linear(3),
        ),
      );
      await tester.pumpAndSettle();

      final timestampFinder = find.byKey(
        const ValueKey('system-message-timestamp-alice'),
      );
      expect(timestampFinder, findsNothing);
      final card = find.byKey(const ValueKey('activity-card-sys-accessible'));
      final icon = find.descendant(of: card, matching: find.byType(Icon));
      expect(icon, findsNothing);
      expect(tester.takeException(), isNull);
    },
  );

  for (final thread in [false, true]) {
    testWidgets(
      'status bar tap reaches oldest content, thread=$thread',
      (tester) async {
        final events = [
          _textMsg(
            id: threadIdTopDashRoot,
            pubkey: 'bob',
            content: 'Oldest content',
            createdAt: 900,
          ),
          for (var index = 0; index < 60; index++)
            _textMsg(
              id: 'top-$index',
              pubkey: 'alice',
              content: 'Message $index',
              createdAt: 1000 + index,
              extraTags: thread
                  ? const [
                      ['e', threadIdTopDashRoot, '', 'reply'],
                    ]
                  : const [],
            ),
        ];
        final messages = formatTimeline(events);
        await tester.pumpWidget(
          _buildTestable(
            messages: events,
            home: thread
                ? ThreadDetailPage(
                    threadHead: messages.firstWhere(
                      (m) => m.id == threadIdTopDashRoot,
                    ),
                    allMessages: messages,
                    channelId: _channelId,
                    currentPubkey: 'me',
                    isMember: true,
                    isArchived: false,
                  )
                : null,
          ),
        );
        await tester.pumpAndSettle();
        final list = tester.widget<ScrollablePositionedList>(
          find.byKey(
            ValueKey(thread ? 'thread-message-list' : 'channel-message-list'),
          ),
        );
        list.itemScrollController!.jumpTo(index: thread ? 55 : 0);
        await tester.pumpAndSettle();
        expect(find.text('Oldest content').hitTestable(), findsNothing);
        await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
          'flutter/status_bar',
          const JSONMethodCodec().encodeMethodCall(
            const MethodCall('handleScrollToTop'),
          ),
          (_) {},
        );
        await tester.pumpAndSettle();
        expect(find.text('Oldest content').hitTestable(), findsOneWidget);
      },
      variant: TargetPlatformVariant.only(TargetPlatform.iOS),
    );
  }

  testWidgets('newest short message keeps the same group gap', (tester) async {
    await tester.pumpWidget(
      _buildTestable(
        messages: [
          for (var index = 0; index < 3; index++)
            _textMsg(
              id: 'gap-$index',
              pubkey: 'alice',
              content: 'Short',
              createdAt: 1000 + index,
            ),
        ],
      ),
    );
    await tester.pumpAndSettle();
    Rect bubble(int index) => tester.getRect(
      find.descendant(
        of: find.byKey(ValueKey('message-row-gap-$index')),
        matching: find.byKey(const ValueKey('message-text-bubble-0')),
      ),
    );
    final firstGap = bubble(1).top - bubble(0).bottom;
    final newestGap = bubble(2).top - bubble(1).bottom;
    expect(newestGap, closeTo(firstGap, 0.01));
  });

  testWidgets('newest short thread reply keeps the same group gap', (
    tester,
  ) async {
    final events = [
      _textMsg(
        id: threadIdRoot,
        pubkey: 'bob',
        content: 'Root',
        createdAt: 900,
      ),
      for (var index = 0; index < 3; index++)
        _textMsg(
          id: 'gap-$index',
          pubkey: 'alice',
          content: 'Short',
          createdAt: 1000 + index,
          extraTags: const [
            ['e', threadIdRoot, '', 'reply'],
          ],
        ),
    ];
    final messages = formatTimeline(events);
    await tester.pumpWidget(
      _buildTestable(
        messages: events,
        home: ThreadDetailPage(
          threadHead: messages.firstWhere((m) => m.id == threadIdRoot),
          allMessages: messages,
          channelId: _channelId,
          currentPubkey: 'me',
          isMember: true,
          isArchived: false,
        ),
      ),
    );
    await tester.pumpAndSettle();
    Rect bubble(int index) => tester.getRect(
      find.descendant(
        of: find.byKey(ValueKey('thread-message-row-gap-$index')),
        matching: find.byKey(const ValueKey('message-text-bubble-0')),
      ),
    );
    expect(
      bubble(2).top - bubble(1).bottom,
      closeTo(bubble(1).top - bubble(0).bottom, 0.01),
    );
  });

  testWidgets('channel groups keep one avatar on the last message', (
    tester,
  ) async {
    final notifier = _FakeMessagesNotifier([
      _textMsg(id: 'first', pubkey: 'alice', content: 'First'),
      _textMsg(id: 'last', pubkey: 'alice', content: 'Last', createdAt: 1010),
    ]);
    await tester.pumpWidget(
      _buildTestable(messages: const [], messagesNotifier: notifier),
    );
    await tester.pumpAndSettle();
    final first = find.byKey(const ValueKey('message-row-first'));
    final last = find.byKey(const ValueKey('message-row-last'));
    expect(
      find.descendant(of: first, matching: find.byType(AvatarImage)),
      findsNothing,
    );
    expect(
      find.descendant(of: last, matching: find.byType(AvatarImage)),
      findsOneWidget,
    );
    expect(find.byKey(const ValueKey('message-author-first')), findsOneWidget);
    expect(find.byKey(const ValueKey('message-author-last')), findsNothing);
    expect(
      find.byWidgetPredicate(
        (widget) =>
            widget.key is ValueKey<String> &&
            (widget.key! as ValueKey<String>).value.startsWith(
              'message-text-bubble-',
            ),
      ),
      findsNWidgets(2),
    );
  });

  testWidgets('live appends move the group avatar and author changes end it', (
    tester,
  ) async {
    final first = _textMsg(id: 'first', pubkey: 'alice', content: 'First');
    final next = _textMsg(
      id: 'next',
      pubkey: 'alice',
      content: 'Next',
      createdAt: 1010,
    );
    final other = _textMsg(
      id: 'other',
      pubkey: 'bob',
      content: 'Other',
      createdAt: 1020,
    );
    final notifier = _FakeMessagesNotifier([first]);
    await tester.pumpWidget(
      _buildTestable(messages: const [], messagesNotifier: notifier),
    );
    await tester.pumpAndSettle();
    Finder avatar(String id) => find.descendant(
      of: find.byKey(ValueKey('message-row-$id')),
      matching: find.byType(AvatarImage),
    );
    expect(avatar('first'), findsOneWidget);
    notifier.setMessages([first, next]);
    await tester.pumpAndSettle();
    expect(avatar('first'), findsNothing);
    expect(avatar('next'), findsOneWidget);
    notifier.setMessages([first, next, other]);
    await tester.pumpAndSettle();
    expect(avatar('next'), findsOneWidget);
    expect(avatar('other'), findsOneWidget);
    notifier.setMessages([first]);
    await tester.pumpAndSettle();
    expect(avatar('first'), findsOneWidget);
  });

  for (final gap in [301, 86400]) {
    testWidgets('author groups end across a $gap second gap', (tester) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(id: 'first', pubkey: 'alice', content: 'First'),
            _textMsg(
              id: 'next',
              pubkey: 'alice',
              content: 'Next',
              createdAt: 1000 + gap,
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();
      for (final id in ['first', 'next']) {
        final row = find.byKey(ValueKey('message-row-$id'));
        expect(
          find.descendant(of: row, matching: find.byType(AvatarImage)),
          findsOneWidget,
        );
        expect(find.byKey(ValueKey('message-author-$id')), findsOneWidget);
      }
    });
  }

  testWidgets(
    'thread replies use the same bubble and group-end avatar layout',
    (tester) async {
      final events = [
        _textMsg(id: threadIdRoot, pubkey: 'bob', content: 'Root'),
        _textMsg(
          id: 'first',
          pubkey: 'alice',
          content: 'First reply',
          createdAt: 1010,
          extraTags: const [
            ['e', threadIdRoot, '', 'reply'],
          ],
        ),
        _textMsg(
          id: 'last',
          pubkey: 'alice',
          content: 'Last reply',
          createdAt: 1020,
          extraTags: const [
            ['e', threadIdRoot, '', 'reply'],
          ],
        ),
      ];
      final messages = formatTimeline(events);
      await tester.pumpWidget(
        _buildTestable(
          messages: events,
          home: ThreadDetailPage(
            threadHead: messages.firstWhere((m) => m.id == threadIdRoot),
            allMessages: messages,
            channelId: _channelId,
            currentPubkey: 'me',
            isMember: true,
            isArchived: false,
          ),
        ),
      );
      await tester.pumpAndSettle();
      final first = find.byKey(const ValueKey('thread-message-row-first'));
      final last = find.byKey(const ValueKey('thread-message-row-last'));
      expect(
        find.descendant(of: first, matching: find.byType(AvatarImage)),
        findsNothing,
      );
      expect(
        find.descendant(of: last, matching: find.byType(AvatarImage)),
        findsOneWidget,
      );
      expect(
        find.byWidgetPredicate(
          (widget) =>
              widget.key is ValueKey<String> &&
              (widget.key! as ValueKey<String>).value.startsWith(
                'message-text-bubble-',
              ),
        ),
        findsNWidgets(3),
      );
    },
  );
}

class _GroupedReactionActions implements ChannelActions {
  final removed = <String>[];
  final added = <String>[];
  @override
  Future<void> removeReaction(String id, String emoji) async => removed.add(id);
  @override
  Future<void> addReaction(String id, String emoji) async => added.add(id);
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
