part of '../channel_detail_page_test.dart';

void swipeReplyTests() {
  // Solo agents answer at top level, so a Solo chat starts no threads.
  Channel soloChannel() => Channel(
    id: _channelId,
    name: 'Solo',
    channelType: 'stream',
    visibility: 'private',
    labels: const ['buzz.solo'],
    description: '',
    createdBy: 'self',
    createdAt: DateTime(2026),
    memberCount: 2,
    isMember: true,
  );

  testWidgets('Solo chat offers no reply by swipe or message actions', (
    tester,
  ) async {
    final channel = soloChannel();
    final events = [
      _textMsg(id: threadIdRoot, pubkey: 'agent', content: 'Agent answer'),
    ];
    await tester.pumpWidget(
      _buildTestable(
        signingPubkey: 'self',
        messages: events,
        channel: channel,
        personalChat: true,
        organization: Stream.value(const MeGroups()),
        home: ChannelDetailPage(channel: channel),
      ),
    );
    await tester.pumpAndSettle();
    final row = find.byKey(const ValueKey('message-row-$threadIdRoot'));
    final bubble = find
        .descendant(of: row, matching: find.byType(LocalSendBubbleSurface))
        .first;
    final initial = tester.getTopLeft(row);
    final gesture = await tester.startGesture(tester.getCenter(bubble));
    await gesture.moveBy(const Offset(25, 0));
    await gesture.moveBy(const Offset(150, 0));
    await tester.pump();
    expect(tester.getTopLeft(row), initial);
    expect(find.byKey(const ValueKey('swipe-reply-circle')), findsNothing);
    await gesture.up();
    await tester.pumpAndSettle();
    expect(find.byType(ThreadDetailPage), findsNothing);

    await tester.longPress(bubble);
    await tester.pumpAndSettle();
    expect(find.text('Copy link'), findsOneWidget);
    expect(find.text('Reply'), findsNothing);
  });

  testWidgets('Solo thread is read-only', (tester) async {
    final channel = soloChannel();
    final events = [
      _textMsg(id: threadIdRoot, pubkey: 'self', content: 'Old thread head'),
    ];
    final messages = formatTimeline(events);
    await tester.pumpWidget(
      _buildTestable(
        signingPubkey: 'self',
        messages: events,
        channel: channel,
        personalChat: true,
        organization: Stream.value(const MeGroups()),
        home: ThreadDetailPage(
          threadHead: messages.first,
          allMessages: messages,
          channelId: _channelId,
          currentPubkey: 'self',
          isMember: true,
          isArchived: false,
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Old thread head'), findsOneWidget);
    expect(find.byType(ComposeBar), findsNothing);
  });

  for (final thread in [false, true]) {
    for (final own in [false, true]) {
      testWidgets(
        'long press beside the bubble opens no actions, thread=$thread own=$own',
        (tester) async {
          // In the channel the row's tap opens this thread.
          final reply = _textMsg(
            id: threadIdA,
            pubkey: 'bob',
            content: 'A reply',
            createdAt: 1001,
            extraTags: [
              ['e', threadIdRoot, '', 'reply'],
            ],
          );
          final events = [
            _textMsg(
              id: threadIdRoot,
              pubkey: own ? 'self' : 'alice',
              content: 'Hold me',
            ),
            if (!thread) reply,
          ];
          final messages = formatTimeline(events);
          final haptics = <MethodCall>[];
          tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            SystemChannels.platform,
            (call) async {
              if (call.method == 'HapticFeedback.vibrate') haptics.add(call);
              return null;
            },
          );
          addTearDown(
            () => tester.binding.defaultBinaryMessenger
                .setMockMethodCallHandler(SystemChannels.platform, null),
          );
          await tester.pumpWidget(
            _buildTestable(
              signingPubkey: 'self',
              messages: events,
              threadReplies: {
                if (!thread) threadIdRoot: [reply],
              },
              home: thread
                  ? ThreadDetailPage(
                      threadHead: messages.first,
                      allMessages: messages,
                      channelId: _channelId,
                      currentPubkey: 'self',
                      isMember: true,
                      isArchived: false,
                    )
                  : ChannelDetailPage(channel: _testChannel),
            ),
          );
          await tester.pumpAndSettle();
          final row = find.byKey(
            ValueKey(
              '${thread ? 'thread-message-row' : 'message-row'}-$threadIdRoot',
            ),
          );
          final bubble = find
              .descendant(
                of: row,
                matching: find.byType(LocalSendBubbleSurface),
              )
              .first;
          final bubbleRect = tester.getRect(bubble);
          final rowRect = tester.getRect(row);
          // The empty side, away from any avatar.
          final beside = Offset(
            own ? rowRect.left + 2 : rowRect.right - 2,
            bubbleRect.center.dy,
          );
          expect(bubbleRect.contains(beside), isFalse);

          // A refused hold is inert through its release: no actions, no
          // haptic, and no fall-through to the row's tap.
          await tester.longPressAt(beside);
          await tester.pumpAndSettle();
          expect(find.text('Copy text'), findsNothing);
          expect(haptics, isEmpty);
          expect(
            find.byType(ThreadDetailPage),
            thread ? findsOneWidget : findsNothing,
          );

          await tester.longPress(bubble);
          await tester.pumpAndSettle();
          expect(find.text('Copy text'), findsOneWidget);
          Navigator.of(tester.element(find.byType(BottomSheet))).pop();
          await tester.pumpAndSettle();

          // A screen reader has no pointer position to test, so the row
          // offers one long press that works although its centre misses the
          // short bubble at the row's edge.
          final semantics = tester.ensureSemantics();
          expect(bubbleRect.contains(rowRect.center), isFalse);
          final holdable = <SemanticsNode>[];
          void collect(SemanticsNode node) {
            if (node.getSemanticsData().hasAction(SemanticsAction.longPress)) {
              holdable.add(node);
            }
            node.visitChildren((child) {
              collect(child);
              return true;
            });
          }

          collect(tester.getSemantics(row));
          expect(holdable, hasLength(1));
          final node = holdable.single;
          node.owner!.performAction(node.id, SemanticsAction.longPress);
          await tester.pumpAndSettle();
          expect(find.text('Copy text'), findsOneWidget);
          semantics.dispose();
          if (thread) return;

          // A quick tap beside the message still opens its thread.
          Navigator.of(tester.element(find.byType(BottomSheet))).pop();
          await tester.pumpAndSettle();
          expect(find.byType(ThreadDetailPage), findsNothing);
          await tester.tapAt(beside);
          await tester.pumpAndSettle();
          expect(find.byType(ThreadDetailPage), findsOneWidget);
          expect(find.text('Copy text'), findsNothing);
        },
      );
    }
  }

  // Every painted part of a message opens its actions; the unused row width
  // on the same line as that part does not, and only bubbles and media start
  // a reply swipe.
  const firstImage = 'https://example.com/media/first.png';
  const secondImage = 'https://example.com/media/second.png';
  const imageTags = [
    ['imeta', 'url $firstImage', 'm image/png'],
    ['imeta', 'url $secondImage', 'm image/png'],
  ];
  final messageParts =
      <
        ({
          String name,
          String content,
          List<List<String>> tags,
          bool edited,
          bool replied,
          bool? thread,
          bool? own,
          bool? swipes,
          Finder Function(Finder row, bool thread) part,
        })
      >[
        (
          name: 'author line',
          content: 'Hold me',
          tags: const [],
          edited: false,
          replied: false,
          thread: null,
          own: null,
          swipes: false,
          part: (row, thread) => find.byKey(
            ValueKey(
              '${thread ? 'thread-message' : 'message'}-timestamp-$threadIdRoot',
            ),
          ),
        ),
        (
          name: 'avatar',
          content: 'Hold me',
          tags: const [],
          edited: false,
          replied: false,
          thread: null,
          own: false,
          swipes: false,
          part: (row, _) => find.descendant(
            of: row,
            matching: find.byType(MessageMediaAvatar),
          ),
        ),
        (
          name: 'photo count label',
          content: 'Gallery\n![First]($firstImage)\n![Second]($secondImage)',
          tags: imageTags,
          edited: false,
          replied: false,
          thread: null,
          own: null,
          swipes: false,
          part: (row, _) => find.descendant(
            of: row,
            matching: find.byKey(const ValueKey('message-media-stack-count')),
          ),
        ),
        (
          name: 'trailing gallery',
          content: 'Gallery\n![First]($firstImage)\n![Second]($secondImage)',
          tags: imageTags,
          edited: false,
          replied: false,
          thread: null,
          own: null,
          // The stack's own horizontal drag pages its photos.
          swipes: null,
          part: (row, _) => find.descendant(
            of: row,
            matching: find.byKey(const ValueKey('message-media-stack')),
          ),
        ),
        (
          name: 'edited marker below media',
          content: 'Gallery\n![First]($firstImage)\n![Second]($secondImage)',
          tags: imageTags,
          edited: true,
          replied: false,
          thread: null,
          own: null,
          swipes: false,
          part: (row, _) => find.descendant(
            of: row,
            matching: find.byType(EditedMarkerBelow),
          ),
        ),
        (
          name: 'attachment',
          content: 'Before\n![First]($firstImage)\nAfter',
          tags: [imageTags.first],
          edited: false,
          replied: false,
          thread: null,
          own: null,
          swipes: true,
          part: (row, _) => find.descendant(
            of: row,
            matching: find.byType(MessageAttachmentCorners),
          ),
        ),
        (
          name: 'reply footer',
          content: 'Hold me',
          tags: const [],
          edited: false,
          replied: true,
          thread: false,
          own: null,
          swipes: false,
          part: (row, _) => find.descendant(
            of: row,
            matching: find.byKey(
              const ValueKey('thread-summary-$threadIdRoot'),
            ),
          ),
        ),
        (
          name: 'add reaction button',
          content: 'Hold me',
          tags: const [],
          edited: false,
          replied: false,
          thread: true,
          own: null,
          swipes: false,
          part: (row, _) => find.descendant(
            of: row,
            matching: find.byKey(const ValueKey('add-reaction-pill')),
          ),
        ),
      ];
  for (final thread in [false, true]) {
    for (final own in [false, true]) {
      for (final part in messageParts) {
        if ((part.thread ?? thread) != thread || (part.own ?? own) != own) {
          continue;
        }
        testWidgets(
          'long press on the ${part.name} opens actions, thread=$thread own=$own',
          (tester) async {
            final author = own ? 'self' : 'alice';
            final events = [
              _textMsg(
                id: threadIdRoot,
                pubkey: author,
                content: part.content,
                extraTags: part.tags,
              ),
              if (part.edited)
                NostrEvent(
                  id: 'edit-1',
                  pubkey: author,
                  createdAt: 1100,
                  kind: EventKind.streamMessageEdit,
                  tags: const [
                    ['h', _channelId],
                    ['e', threadIdRoot],
                  ],
                  content: part.content,
                  sig: '',
                ),
              if (part.replied)
                _textMsg(
                  id: 'reply-1',
                  pubkey: 'bob',
                  content: 'First reply',
                  createdAt: 1200,
                  extraTags: const [
                    ['e', threadIdRoot, '', 'reply'],
                  ],
                ),
            ];
            final messages = formatTimeline(events);
            await tester.pumpWidget(
              _buildTestable(
                signingPubkey: 'self',
                messages: events,
                users: const {
                  'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
                },
                home: thread
                    ? ThreadDetailPage(
                        threadHead: messages.first,
                        allMessages: messages,
                        channelId: _channelId,
                        currentPubkey: 'self',
                        isMember: true,
                        isArchived: false,
                      )
                    : ChannelDetailPage(channel: _testChannel),
              ),
            );
            await tester.pumpAndSettle();
            final row = find.byKey(
              ValueKey(
                '${thread ? 'thread-message-row' : 'message-row'}-$threadIdRoot',
              ),
            );
            final target = part.part(row, thread);
            expect(target, findsOneWidget);
            final partRect = tester.getRect(target);
            final rowRect = tester.getRect(row);
            await tester.longPressAt(partRect.center);
            await tester.pumpAndSettle();
            expect(find.text('Copy link'), findsOneWidget);
            Navigator.of(tester.element(find.byType(BottomSheet))).pop();
            await tester.pumpAndSettle();

            // Only bubbles and media start a reply swipe.
            final swipes = part.swipes;
            if (swipes != null) {
              final initial = tester.getTopLeft(row);
              final drag = await tester.startGesture(partRect.center);
              await drag.moveBy(const Offset(-25, 0));
              await drag.moveBy(const Offset(-40, 0));
              await tester.pump();
              expect(
                tester.getTopLeft(row).dx,
                swipes ? lessThan(initial.dx) : initial.dx,
              );
              await drag.up();
              await tester.pumpAndSettle();
            }

            // The unused row width on the part's own line, away from any
            // avatar.
            final beside = Offset(
              own ? rowRect.left + 2 : rowRect.right - 2,
              partRect.center.dy,
            );
            expect(partRect.contains(beside), isFalse);
            await tester.longPressAt(beside);
            await tester.pumpAndSettle();
            expect(find.text('Copy link'), findsNothing);
            // Its release is no tap on the row, so no thread opens.
            expect(
              find.byType(ThreadDetailPage),
              thread ? findsOneWidget : findsNothing,
            );

            // The gallery's reply-drag target spans the row, so the blank
            // strip beside its deck still starts a reply swipe.
            if (part.name == 'trailing gallery') {
              final initial = tester.getTopLeft(row);
              final drag = await tester.startGesture(beside);
              await drag.moveBy(const Offset(-25, 0));
              await drag.moveBy(const Offset(-40, 0));
              await tester.pump();
              expect(tester.getTopLeft(row).dx, lessThan(initial.dx));
              await drag.up();
              await tester.pumpAndSettle();
            }

            // The part's own tap still wins over the message.
            if (part.name == 'reply footer') {
              await tester.tapAt(partRect.center);
              await tester.pumpAndSettle();
              expect(find.byType(ThreadDetailPage), findsOneWidget);
            } else if (part.name == 'avatar') {
              await tester.tapAt(partRect.center);
              await tester.pumpAndSettle();
              expect(find.byType(UserProfileSheet), findsOneWidget);
            }
          },
        );
      }
    }
  }

  for (final thread in [false, true]) {
    for (final own in [false, true]) {
      testWidgets(
        'left drag opens existing reply destination, thread=$thread own=$own',
        (tester) async {
          final events = [
            _textMsg(
              id: threadIdRoot,
              pubkey: own ? 'self' : 'alice',
              content: 'Swipe this message\nA second line\nAnd a third line',
            ),
          ];
          final messages = formatTimeline(events);
          var navigationDrags = 0;
          await tester.pumpWidget(
            _buildTestable(
              signingPubkey: 'self',
              messages: events,
              chatColor: 'magenta',
              home: GestureDetector(
                onHorizontalDragUpdate: (_) => navigationDrags++,
                child: thread
                    ? ThreadDetailPage(
                        threadHead: messages.first,
                        allMessages: messages,
                        channelId: _channelId,
                        currentPubkey: 'self',
                        isMember: true,
                        isArchived: false,
                      )
                    : ChannelDetailPage(channel: _testChannel),
              ),
            ),
          );
          await tester.pumpAndSettle();
          final row = find.byKey(
            ValueKey(
              '${thread ? 'thread-message-row' : 'message-row'}-$threadIdRoot',
            ),
          );
          final initial = tester.getTopLeft(row);
          final haptics = <MethodCall>[];
          tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
            SystemChannels.platform,
            (call) async {
              if (call.method == 'HapticFeedback.vibrate') haptics.add(call);
              return null;
            },
          );
          addTearDown(
            () => tester.binding.defaultBinaryMessenger
                .setMockMethodCallHandler(SystemChannels.platform, null),
          );
          // Metadata, avatar and empty row space never arm a reply, even when
          // the pointer later crosses into the message.
          final bubbleRect = tester.getRect(
            find
                .descendant(
                  of: row,
                  matching: find.byType(LocalSendBubbleSurface),
                )
                .first,
          );
          for (final start in [
            Offset(tester.getRect(row).left + 2, bubbleRect.center.dy),
            Offset(tester.getRect(row).right - 2, bubbleRect.center.dy),
            Offset(bubbleRect.left + 4, tester.getRect(row).top + 2),
          ]) {
            if (bubbleRect.contains(start)) continue;
            final previousNavigationDrags = navigationDrags;
            final outside = await tester.startGesture(start);
            await outside.moveBy(const Offset(-25, 0));
            await outside.moveBy(const Offset(-100, 0));
            await tester.pump();
            expect(tester.getTopLeft(row), initial);
            expect(haptics, isEmpty);
            expect(navigationDrags, greaterThan(previousNavigationDrags));
            await outside.up();
            await tester.pumpAndSettle();
            expect(Navigator.of(tester.element(row)).canPop(), isFalse);
          }
          // A rightward drag on the bubble is left to swipe-back navigation.
          final bubbleCenter = bubbleRect.center;
          final previousNavigationDrags = navigationDrags;
          final back = await tester.startGesture(bubbleCenter);
          await back.moveBy(const Offset(25, 0));
          await back.moveBy(const Offset(100, 0));
          await tester.pump();
          expect(tester.getTopLeft(row), initial);
          expect(haptics, isEmpty);
          expect(navigationDrags, greaterThan(previousNavigationDrags));
          await back.up();
          await tester.pumpAndSettle();
          expect(Navigator.of(tester.element(row)).canPop(), isFalse);
          final gesture = await tester.startGesture(bubbleCenter);
          await gesture.moveBy(const Offset(-25, 0));
          await gesture.moveBy(const Offset(-40, 0));
          await tester.pump();
          expect(tester.getTopLeft(row).dx, lessThan(initial.dx));
          expect(haptics, isEmpty);
          final partialCircle = tester.widget<DecoratedBox>(
            find.byKey(const ValueKey('swipe-reply-circle')),
          );
          expect(
            (partialCircle.decoration as BoxDecoration).color,
            Colors.transparent,
          );
          final ring = tester.widget<CircularProgressIndicator>(
            find.descendant(
              of: find.byKey(const ValueKey('swipe-reply-circle')),
              matching: find.byType(CircularProgressIndicator),
            ),
          );
          expect(ring.strokeWidth, 2);
          expect(ring.value, inExclusiveRange(0, 1));
          await gesture.moveBy(const Offset(-45, 0));
          await tester.pump();
          expect(haptics, hasLength(1));
          await gesture.moveBy(const Offset(-10, 0));
          await tester.pump();
          expect(haptics, hasLength(1));
          final armedPosition = tester.getTopLeft(row);
          await gesture.moveBy(const Offset(-160, 0));
          await tester.pump();
          final resistedPosition = tester.getTopLeft(row);
          expect(resistedPosition.dx, lessThan(armedPosition.dx));
          expect(resistedPosition.dx, greaterThan(armedPosition.dx - 40));
          final circle = find.byKey(const ValueKey('swipe-reply-circle'));
          final circleRect = tester.getRect(circle);
          // The armed slide uncovers the feedback, which stays on screen.
          expect(
            circleRect.left,
            greaterThan(
              tester
                  .getRect(
                    find
                        .descendant(
                          of: row,
                          matching: find.byType(LocalSendBubbleSurface),
                        )
                        .first,
                  )
                  .right,
            ),
          );
          expect(
            circleRect.right,
            lessThanOrEqualTo(
              tester.view.physicalSize.width / tester.view.devicePixelRatio,
            ),
          );
          if (!own) {
            // Incoming bubbles leave room for the usual gutter.
            expect(circleRect.left, closeTo(bubbleRect.right + Grid.xxs, 0.01));
          }
          final fill =
              tester.widget<DecoratedBox>(circle).decoration as BoxDecoration;
          // The armed circle uses the chosen message color, not the app accent.
          expect(fill.color, const Color(0xFFCC1F7A));
          expect(find.text('Reply'), findsOneWidget);
          final centerBeforePulse = tester.getCenter(
            find.byKey(const ValueKey('swipe-reply-feedback')),
          );
          await tester.pump(const Duration(milliseconds: 60));
          expect(
            tester.getCenter(
              find.byKey(const ValueKey('swipe-reply-feedback')),
            ),
            centerBeforePulse,
          );
          expect(tester.getRect(circle).width, greaterThan(Grid.lg));
          await tester.pump(const Duration(milliseconds: 180));
          expect(tester.getRect(circle).width, Grid.lg);
          expect(haptics, hasLength(1));
          final feedback = find.byKey(const ValueKey('swipe-reply-feedback'));
          expect(tester.getCenter(feedback).dy, bubbleRect.center.dy);
          expect(
            tester.getCenter(find.text('Reply')).dx,
            tester.getCenter(circle).dx,
          );
          expect(
            tester.widget<Text>(find.text('Reply')).style?.color,
            tester.element(circle).colors.onSurfaceVariant,
          );
          // Retreat below readiness without returning to rest, then cross again.
          await gesture.moveBy(const Offset(210, 0));
          await tester.pump();
          expect(tester.getTopLeft(row).dx, lessThan(initial.dx));
          await gesture.moveBy(const Offset(-45, 0));
          await tester.pump();
          expect(haptics, hasLength(2));
          await tester.pump(const Duration(milliseconds: 60));
          expect(tester.getRect(circle).width, greaterThan(Grid.lg));
          await tester.pump(const Duration(milliseconds: 180));
          expect(tester.getRect(circle).width, Grid.lg);
          await gesture.up();
          await tester.pumpAndSettle();
          expect(find.byType(ThreadDetailPage), findsOneWidget);
          expect(
            tester
                .widget<ThreadDetailPage>(find.byType(ThreadDetailPage))
                .threadHead
                .id,
            threadIdRoot,
          );
          // A thread message must push a reply route too, rather than doing nothing.
          expect(
            Navigator.of(
              tester.element(find.byType(ThreadDetailPage)),
            ).canPop(),
            isTrue,
          );
        },
      );
    }
  }

  for (final movement in [
    const Offset(-45, 0),
    const Offset(100, 0),
    const Offset(0, -100),
  ]) {
    testWidgets('non-reply drag $movement does not navigate', (tester) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _textMsg(
              id: 'swipe-cancel',
              pubkey: 'alice',
              content: 'Keep reading',
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();
      final row = find.byKey(const ValueKey('message-row-swipe-cancel'));
      await tester.drag(
        find
            .descendant(of: row, matching: find.byType(LocalSendBubbleSurface))
            .first,
        movement,
      );
      await tester.pumpAndSettle();
      expect(find.byType(ThreadDetailPage), findsNothing);
    });
  }

  for (final reducedMotion in [false, true]) {
    testWidgets('cancelled drag resets feedback, reduced=$reducedMotion', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          disableAnimations: reducedMotion,
          messages: [
            _textMsg(id: 'cancel', pubkey: 'alice', content: 'Cancel drag'),
          ],
        ),
      );
      await tester.pumpAndSettle();
      final row = find.byKey(const ValueKey('message-row-cancel'));
      final origin = tester.getTopLeft(row);
      final gesture = await tester.startGesture(
        tester.getCenter(
          find
              .descendant(
                of: row,
                matching: find.byType(LocalSendBubbleSurface),
              )
              .first,
        ),
      );
      await gesture.moveBy(const Offset(-25, 0));
      await gesture.moveBy(const Offset(-90, 0));
      await tester.pump();
      final icon = find.byIcon(BuzzIcons.messageSquareReply);
      expect(
        tester
            .widget<Opacity>(
              find.ancestor(of: icon, matching: find.byType(Opacity)).first,
            )
            .opacity,
        1,
      );
      await gesture.cancel();
      await tester.pumpAndSettle();
      expect(tester.getTopLeft(row), origin);
      expect(find.byType(ThreadDetailPage), findsNothing);
    });
  }

  testWidgets('crossing threshold then retreating cancels reply', (
    tester,
  ) async {
    await tester.pumpWidget(
      _buildTestable(
        messages: [
          _textMsg(id: 'retreat', pubkey: 'alice', content: 'Keep reading'),
        ],
      ),
    );
    await tester.pumpAndSettle();
    final row = find.byKey(const ValueKey('message-row-retreat'));
    final gesture = await tester.startGesture(
      tester.getCenter(
        find
            .descendant(of: row, matching: find.byType(LocalSendBubbleSurface))
            .first,
      ),
    );
    await gesture.moveBy(const Offset(-25, 0));
    await gesture.moveBy(const Offset(-90, 0));
    await gesture.moveBy(const Offset(70, 0));
    await gesture.up();
    await tester.pumpAndSettle();
    expect(find.byType(ThreadDetailPage), findsNothing);
  });
}

void swipeReplyLinkTests() {
  for (final onLink in [true, false]) {
    testWidgets(
      'left drag starting ${onLink ? 'on a link' : 'beside a link'} replies',
      variant: const TargetPlatformVariant({
        TargetPlatform.iOS,
        TargetPlatform.android,
      }),
      (tester) async {
        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _textMsg(
                id: 'swipe-link',
                pubkey: 'alice',
                content: 'Read [the doc](https://example.com/doc) carefully',
              ),
            ],
          ),
        );
        await tester.pumpAndSettle();
        final gesture = await tester.startGesture(
          onLink
              ? linkCenter(tester, 'the doc')
              : textCenter(tester, 'carefully'),
        );
        await gesture.moveBy(const Offset(-25, 0));
        await gesture.moveBy(const Offset(-100, 0));
        await tester.pump();
        await gesture.up();
        await tester.pumpAndSettle();
        expect(find.byType(ThreadDetailPage), findsOneWidget);
        expect(find.text('Copy link'), findsNothing);
      },
    );
  }
}
