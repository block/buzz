part of '../channel_detail_page_test.dart';

void actionRowTests() {
  const users = {
    'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
    'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
  };
  final cases = [
    (
      payload: <String, dynamic>{'type': 'member_removed', 'target': 'bob'},
      action: 'removed Bob from the channel',
    ),
    (
      payload: <String, dynamic>{'type': 'member_removed'},
      action: 'removed Someone from the channel',
    ),
    for (final field in ['topic']) ...[
      (
        payload: <String, dynamic>{
          'type': '${field}_changed',
          field: '  Release planning  ',
        },
        action: 'changed the $field to "Release planning"',
      ),
      for (final blank in [null, '', ' \n\t '])
        (
          payload: <String, dynamic>{'type': '${field}_changed', field: blank},
          action: 'cleared the $field',
        ),
    ],
  ];

  group('Channel action rows', () {
    for (var index = 0; index < cases.length; index++) {
      final scenario = cases[index];
      testWidgets('uses compact activity layout for action $index', (
        tester,
      ) async {
        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _textMsg(
                id: 'regular',
                pubkey: 'bob',
                content: 'Regular message',
                createdAt: 1000,
              ),
              _systemMsg(
                id: 'action',
                payload: {...scenario.payload, 'actor': 'alice'},
                createdAt: 1010,
              ),
              _reaction(id: 'reaction', targetId: 'action'),
            ],
            users: users,
          ),
        );
        await tester.pumpAndSettle();

        final row = find.byKey(const ValueKey('system-message-row-action'));
        final avatars = find.descendant(
          of: row,
          matching: find.byType(CircleAvatar),
        );
        expect(avatars, findsNothing);
        final author = find.byKey(
          const ValueKey('system-message-author-alice'),
        );
        expect(author, findsOneWidget);
        expect(
          find.byKey(const ValueKey('system-message-author-bob')),
          scenario.payload['target'] == 'bob' ? findsOneWidget : findsNothing,
        );
        expect(findRichText('Alice ${scenario.action}'), findsNothing);
        final body = findRichText(
          scenario.payload['target'] == 'bob' ? 'removed ' : scenario.action,
        );
        if (scenario.payload['target'] == 'bob') {
          expect(
            find.byKey(const ValueKey('system-message-author-bob')),
            findsOneWidget,
          );
          expect(find.text('@'), findsNothing);
        }
        expect(body, findsOneWidget);
        final regularBody = tester.getRect(findRichText('Regular message'));
        final regularAuthor = tester.getRect(
          find.byKey(const ValueKey('message-author-regular')),
        );
        final actionBody = tester.getRect(body);
        final actionAuthor = tester.getRect(author);
        final card = tester.getRect(
          find.byKey(const ValueKey('activity-card-action')),
        );
        expect(card.center.dx, closeTo(tester.getRect(row).center.dx, 0.01));
        expect(actionAuthor.left, greaterThanOrEqualTo(actionBody.left));
        expect(regularAuthor.left, regularBody.left);
        expect(actionAuthor.top, greaterThanOrEqualTo(actionBody.top));
        final timestamp = find.byKey(
          const ValueKey('system-message-timestamp-alice'),
        );
        expect(timestamp, findsNothing);
        final reactions = find.descendant(
          of: row,
          matching: find.byType(ReactionRow),
        );
        expect(reactions, findsOneWidget);
        expect(tester.getCenter(reactions).dx, closeTo(card.center.dx, 0.01));
        expect(tester.takeException(), isNull);
      });

      testWidgets('keeps missing-actor fallback for action $index', (
        tester,
      ) async {
        await tester.pumpWidget(
          _buildTestable(
            messages: [_systemMsg(id: 'fallback', payload: scenario.payload)],
            users: users,
          ),
        );
        await tester.pumpAndSettle();
        expect(findRichText('Someone ${scenario.action}'), findsOneWidget);
        expect(
          find.byIcon(
            scenario.payload['type'] == 'member_removed'
                ? BuzzIcons.userMinus
                : BuzzIcons.pencil,
          ),
          findsNothing,
        );
        expect(
          find.byKey(const ValueKey('system-message-timestamp-fallback')),
          findsNothing,
        );
      });
    }

    testWidgets('removal keeps both profiles and reaction actions accessible', (
      tester,
    ) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            _systemMsg(
              id: 'removal',
              payload: {
                'type': 'member_removed',
                'actor': 'alice',
                'target': 'bob',
              },
            ),
          ],
          users: users,
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(
        find.byKey(const ValueKey('system-message-author-alice')),
      );
      await tester.pumpAndSettle();
      expect(
        tester.widget<UserProfileSheet>(find.byType(UserProfileSheet)).pubkey,
        'alice',
      );
      await tester.tap(find.byTooltip('Close sheet'));
      await tester.pumpAndSettle();
      final targetSemantics = find.semantics.byLabel('Bob').evaluate();
      expect(targetSemantics, hasLength(1));
      expect(
        targetSemantics.single.getSemanticsData().hasAction(
          SemanticsAction.tap,
        ),
        isTrue,
      );
      await tester.tap(find.byKey(const ValueKey('system-message-author-bob')));
      await tester.pumpAndSettle();
      expect(
        tester.widget<UserProfileSheet>(find.byType(UserProfileSheet)).pubkey,
        'bob',
      );
      await tester.tap(find.byTooltip('Close sheet'));
      await tester.pumpAndSettle();
      await tester.longPress(
        find.byKey(const ValueKey('system-message-row-removal')),
      );
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('quick-reaction-more')), findsOneWidget);
      Navigator.of(tester.element(find.byType(BottomSheet))).pop();
      await tester.pumpAndSettle();
    });

    for (final source in ['directory', 'profile', 'channel role']) {
      testWidgets('removed agent has a plain tappable name from $source', (
        tester,
      ) async {
        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _systemMsg(
                id: 'agent-removal',
                payload: {
                  'type': 'member_removed',
                  'actor': 'alice',
                  'target': 'bob',
                },
              ),
            ],
            users: {
              ...users,
              'bob': UserProfile(
                pubkey: 'bob',
                displayName: 'Helper Bot',
                ownerPubkey: source == 'profile' ? 'alice' : null,
              ),
            },
            knownAgentPubkeys: source == 'directory' ? {'bob'} : {},
            loadChannelBotPubkeys: () async =>
                source == 'channel role' ? {'bob'} : {},
          ),
        );
        await tester.pumpAndSettle();
        final pill = find.byKey(const ValueKey('system-message-author-bob'));
        expect(pill, findsOneWidget);
        expect(find.byType(MessageMentionPill), findsNothing);
        expect(
          tester.widget<Text>(find.text('Helper Bot')).style?.decoration,
          isNot(TextDecoration.underline),
        );
        final target = find.semantics.byLabel('Helper Bot').evaluate();
        expect(target, hasLength(1));
        expect(
          target.single.getSemanticsData().hasAction(SemanticsAction.tap),
          isTrue,
        );
        await tester.tap(pill);
        await tester.pumpAndSettle();
        expect(
          tester.widget<UserProfileSheet>(find.byType(UserProfileSheet)).pubkey,
          'bob',
        );
      });
    }

    testWidgets(
      'agent headers, action targets, and profiles keep distinct identities',
      (tester) async {
        final first = 'a' * 64, second = 'b' * 64;
        final agents = {first, second};
        final profiles = {
          for (final key in agents)
            key: UserProfile(pubkey: key, displayName: 'Scout'),
        };
        final names = IdentityNameSources(
          profiles: profiles,
          agentPubkeys: agents,
        ).scope(agents);
        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _textMsg(id: 'agent-message', pubkey: first, content: 'hello'),
              _systemMsg(
                id: 'agent-removal',
                payload: {
                  'type': 'member_removed',
                  'actor': first,
                  'target': second,
                },
              ),
            ],
            users: profiles,
            knownAgentPubkeys: agents,
            members: [
              for (final key in agents)
                ChannelMember(
                  pubkey: key,
                  role: 'bot',
                  joinedAt: DateTime(2025),
                ),
            ],
          ),
        );
        await tester.pumpAndSettle();
        final header = find.byKey(
          const ValueKey('message-author-agent-message'),
        );
        expect(tester.widget<Text>(header).data, names.labelFor(first));
        expect(
          tester
              .widget<Text>(
                find.byKey(ValueKey('system-message-author-$first')),
              )
              .data,
          names.labelFor(first),
        );
        final pill = find.byKey(ValueKey('system-message-author-$second'));
        expect(pill, findsOneWidget);
        expect(names.labelFor(first), isNot(names.labelFor(second)));
        expect(find.text('Scout'), findsNothing);
        for (final entry in [(header, first), (pill, second)]) {
          await tester.tap(entry.$1);
          await tester.pumpAndSettle();
          final sheet = find.byType(UserProfileSheet);
          expect(tester.widget<UserProfileSheet>(sheet).pubkey, entry.$2);
          expect(
            find.descendant(
              of: sheet,
              matching: find.text(names.resolve(entry.$2)!.name),
            ),
            findsOneWidget,
          );
          await tester.tap(find.byTooltip('Close sheet'));
          await tester.pumpAndSettle();
        }
        expect(tester.takeException(), isNull);
      },
    );

    for (final scenario in [cases.first, cases[2]]) {
      testWidgets('wraps ${scenario.payload['type']} at large text sizes', (
        tester,
      ) async {
        tester.view.physicalSize = const Size(320, 800);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        await tester.pumpWidget(
          _buildTestable(
            messages: [
              _systemMsg(
                id: 'large',
                payload: {...scenario.payload, 'actor': 'alice'},
              ),
            ],
            users: users,
            textScaler: const TextScaler.linear(3),
          ),
        );
        await tester.pumpAndSettle();
        expect(
          findRichText(
            scenario.payload['target'] == 'bob' ? 'removed ' : scenario.action,
          ),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('system-message-timestamp-alice')),
          findsNothing,
        );
        expect(tester.takeException(), isNull);
      });
    }
  });
}
