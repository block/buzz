part of '../channel_detail_page_test.dart';

void reviewAvatarTests() {
  for (final payload in [
    '{',
    '{}',
    '{"ephemeral_channel_id":null}',
    '{"ephemeral_channel_id":7}',
    '{"ephemeral_channel_id":""}',
    '{"ephemeral_channel_id":" "}',
    '{"ephemeral_channel_id":"not-a-uuid"}',
    '{"ephemeral_channel_id":"8d764100fd8f44cf9c986d8fbd739b8z"}',
  ]) {
    testWidgets('invalid huddle has fallback and no invitation: $payload', (
      tester,
    ) async {
      final event = NostrEvent(
        id: 'invalid-room',
        pubkey: 'alice',
        createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
        kind: EventKind.huddleStarted,
        tags: const [
          ['h', _channelId],
        ],
        content: payload,
        sig: '',
      );
      await tester.pumpWidget(
        _buildTestable(messages: [event], huddleLifecycle: [event]),
      );
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('activity-card-invalid-room')),
        findsOneWidget,
      );
      expect(findRichText('started a huddle'), findsOneWidget);
      expect(
        find.byKey(const ValueKey('huddle-card-invalid-room')),
        findsNothing,
      );
      expect(find.widgetWithText(FilledButton, 'Join'), findsNothing);
      expect(find.byTooltip('Join Huddle'), findsNothing);
      expect(find.byTooltip('Open Huddle'), findsNothing);
      expect(find.byTooltip('Start Huddle'), findsOneWidget);
    });
  }
  for (final room in [
    _huddleChannelId,
    _huddleChannelId.toUpperCase(),
    _huddleChannelId.replaceAll('-', ''),
    'urn:uuid:$_huddleChannelId',
    '{$_huddleChannelId}',
  ]) {
    testWidgets(
      'relay-valid huddle UUID has row and header invitation: $room',
      (tester) async {
        final event = _huddleMsg(
          id: 'valid-room',
          kind: EventKind.huddleStarted,
          createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
          ephemeralChannelId: room,
        );
        expect(
          SystemEvent.fromHuddleEvent(event)!.ephemeralChannelId,
          _huddleChannelId,
        );
        await tester.pumpWidget(
          _buildTestable(messages: [event], huddleLifecycle: [event]),
        );
        await tester.pumpAndSettle();
        expect(
          find.byKey(const ValueKey('huddle-card-valid-room')),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('huddle-Join-$_huddleChannelId')),
          findsOneWidget,
        );
        expect(find.byTooltip('Open Huddle'), findsOneWidget);
      },
    );
  }
  testWidgets(
    'bounded window retains orphan ending and reads its visible bottom',
    (tester) async {
      final readState = _SynchronousReadStateNotifier(
        const ReadStateState(
          isReady: true,
          pubkey: 'self',
          contexts: {},
          version: 0,
        ),
      );
      final events = [
        _huddleMsg(
          id: 'different-start',
          kind: EventKind.huddleStarted,
          createdAt: 999,
          ephemeralChannelId: '9d764100-fd8f-44cf-9c98-6d8fbd739b8c',
        ),
        for (var i = 0; i < 198; i++)
          _textMsg(
            id: 'window-$i',
            pubkey: 'alice',
            content: 'Message $i',
            createdAt: 1000 + i,
          ),
        _huddleMsg(
          id: 'orphan-end',
          kind: EventKind.huddleEnded,
          createdAt: 1300,
        ),
      ];
      await tester.pumpWidget(
        _buildTestable(messages: events, readStateNotifier: readState),
      );
      await tester.pumpAndSettle();
      expect(findRichText('ended the huddle'), findsOneWidget);
      expect(
        find.byKey(const ValueKey('activity-card-orphan-end')),
        findsOneWidget,
      );
      await tester.pump(const Duration(milliseconds: 300));
      expect(readState.markedContexts['activity:$_channelId'], 1300);
    },
  );
  testWidgets(
    'paired huddle ending with reactions retains its reaction surface',
    (tester) async {
      late _EndingReactionActions actions;
      await tester.pumpWidget(
        _buildTestable(
          createChannelActions: (ref) => actions = _EndingReactionActions(ref),
          messages: [
            _huddleMsg(id: 'paired-start', kind: EventKind.huddleStarted),
            _huddleMsg(
              id: 'reacted-end',
              kind: EventKind.huddleEnded,
              createdAt: 1100,
            ),
            _reaction(
              id: 'end-reaction',
              targetId: 'reacted-end',
              pubkey: 'self',
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('activity-card-reacted-end')),
        findsOneWidget,
      );
      final reactions = tester.widgetList<ReactionRow>(
        find.byType(ReactionRow),
      );
      expect(
        reactions.any(
          (row) =>
              row.messageId == 'reacted-end' &&
              row.reactions.any((reaction) => reaction.emoji == '👍'),
        ),
        isTrue,
      );
      await tester.tap(find.text('👍').first);
      await tester.pump();
      expect(actions.removed, 'end-reaction');
    },
  );
  for (final thread in [false, true]) {
    testWidgets(
      'production gallery profile semantics and caption edit thread=$thread',
      (tester) async {
        final handle = tester.ensureSemantics();
        final alice = 'a' * 64;
        NostrEvent message(String caption, [String second = 'two']) => _textMsg(
          id: threadIdRoot,
          pubkey: alice,
          content:
              '$caption\n![One](https://example.com/one.png)\n![Two](https://example.com/$second.png)',
        );
        final notifier = _FakeMessagesNotifier([message('Original')]);
        await tester.pumpWidget(
          _buildTestable(
            messages: const [],
            messagesNotifier: notifier,
            users: {alice: UserProfile(pubkey: alice, displayName: 'Alice')},
            home: thread
                ? Consumer(
                    builder: (context, ref, _) {
                      final messages = formatTimeline(
                        ref.watch(channelMessagesProvider(_channelId)).value ??
                            [],
                        currentPubkey: 'self',
                      );
                      return ThreadDetailPage(
                        threadHead: messages.first,
                        allMessages: messages,
                        channelId: _channelId,
                        currentPubkey: 'self',
                        isMember: true,
                        isArchived: false,
                      );
                    },
                  )
                : null,
          ),
        );
        await tester.pumpAndSettle();
        final gate = find.byType(MessageMediaAvatar).first;
        final avatar = find
            .descendant(of: gate, matching: find.byType(AvatarImage))
            .first;
        final profile = tester.getSemantics(
          find.bySemanticsLabel('Open Alice profile'),
        );
        expect(profile.getSemanticsData().flagsCollection.isButton, isTrue);
        expect(
          profile.getSemanticsData().hasAction(SemanticsAction.tap),
          isTrue,
        );
        bool profileIsAccessible() {
          var found = false;
          void visit(SemanticsNode node) {
            if (node.id == profile.id &&
                node.getSemanticsData().hasAction(SemanticsAction.tap)) {
              found = true;
            }
            node.visitChildren((child) {
              visit(child);
              return true;
            });
          }

          visit(
            tester
                .binding
                .renderViews
                .single
                .owner!
                .semanticsOwner!
                .rootSemanticsNode!,
          );
          return found;
        }

        final carousel = find.byKey(const ValueKey('message-media-carousel'));
        final controller =
            tester
                    .widget<CustomScrollView>(
                      find.descendant(
                        of: carousel,
                        matching: find.byType(CustomScrollView),
                      ),
                    )
                    .controller!
                as PageController;
        expect(profileIsAccessible(), isTrue);
        controller.jumpTo(-10);
        await tester.pump();
        expect(profileIsAccessible(), isTrue);
        controller.jumpToPage(1);
        await tester.pumpAndSettle();
        expect(profileIsAccessible(), isFalse);
        final pageBeforeEdit = controller.page;
        expect(pageBeforeEdit, greaterThan(0.5));
        notifier.setMessages([message('Edited caption')]);
        await tester.pumpAndSettle();
        expect(controller.page, pageBeforeEdit);
        expect(profileIsAccessible(), isFalse);
        controller.jumpToPage(0);
        await tester.pumpAndSettle();
        // The semantics node may be recreated after exclusion.
        expect(
          tester
              .getSemantics(avatar)
              .getSemanticsData()
              .hasAction(SemanticsAction.tap),
          isTrue,
        );
        controller.jumpToPage(1);
        await tester.pumpAndSettle();
        notifier.setMessages([message('New media', 'replacement')]);
        await tester.pumpAndSettle();
        final replacementController =
            tester
                    .widget<CustomScrollView>(
                      find.descendant(
                        of: carousel,
                        matching: find.byType(CustomScrollView),
                      ),
                    )
                    .controller!
                as PageController;
        expect(replacementController.page, 0);
        final restored = tester.getSemantics(
          find.bySemanticsLabel('Open Alice profile'),
        );
        expect(restored.getSemanticsData().flagsCollection.isButton, isTrue);
        expect(
          restored.getSemanticsData().hasAction(SemanticsAction.tap),
          isTrue,
        );
        tester.binding.renderViews.single.owner!.semanticsOwner!.performAction(
          restored.id,
          SemanticsAction.tap,
        );
        handle.dispose();
        await tester.pumpAndSettle();
        expect(find.byType(UserProfileSheet), findsOneWidget);
        expect(
          find.descendant(
            of: find.byType(UserProfileSheet),
            matching: find.text('Alice'),
          ),
          findsWidgets,
        );
      },
    );
  }
}

class _EndingReactionActions extends _FakeChannelActions {
  _EndingReactionActions(super.ref);
  String? removed;
  @override
  Future<void> removeReaction(String reactionEventId, String emoji) async {
    removed = reactionEventId;
  }
}
