part of '../channel_detail_page_test.dart';

void _channelMemberRowTests() {
  testWidgets(
    'channel details keeps participation implicit and shows exceptional roles',
    (tester) async {
      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          members: [
            ChannelMember(
              pubkey: 'self',
              role: 'owner',
              joinedAt: DateTime(2025),
              displayName: 'Self',
            ),
            ChannelMember(
              pubkey: 'alice',
              role: 'member',
              joinedAt: DateTime(2025),
              displayName: 'Alice',
            ),
            ChannelMember(
              pubkey: 'agent',
              role: 'bot',
              joinedAt: DateTime(2025),
              displayName: 'Agent',
            ),
            ChannelMember(
              pubkey: 'admin',
              role: 'admin',
              joinedAt: DateTime(2025),
              displayName: 'Admin person',
            ),
            ChannelMember(
              pubkey: 'guest',
              role: 'guest',
              joinedAt: DateTime(2025),
              displayName: 'Guest person',
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();

      await tester.tap(
        find.byKey(const ValueKey('channel-header-settings-trigger')),
      );
      await tester.pumpAndSettle();
      expect(find.text('5 members'), findsOneWidget);
      expect(find.text('You · Owner', findRichText: true), findsOneWidget);
      expect(find.text('Alice', findRichText: true), findsOneWidget);
      expect(find.text('Agent', findRichText: true), findsOneWidget);
      expect(
        find.text('Admin person · Admin', findRichText: true),
        findsOneWidget,
      );
      expect(
        find.text('Guest person · Viewing only', findRichText: true),
        findsOneWidget,
      );
      expect(
        find.textContaining('Participating', findRichText: true),
        findsNothing,
      );
      expect(find.text('Member'), findsNothing);
      expect(find.text('Owner'), findsNothing);
      expect(find.text('People · 2'), findsNothing);
      expect(find.text('Agents · 1'), findsNothing);
      expect(find.text('PEOPLE — 2'), findsNothing);
      expect(find.text('BOTS — 1'), findsNothing);

      final aliceRow = find.byKey(
        const ValueKey('channel-details-member-alice'),
      );
      final aliceText = tester.widget<Text>(
        find.descendant(
          of: aliceRow,
          matching: find.byWidgetPredicate(
            (widget) =>
                widget is Text && widget.textSpan?.toPlainText() == 'Alice',
          ),
        ),
      );
      final aliceSpans = (aliceText.textSpan! as TextSpan).children!;
      expect(aliceSpans, hasLength(1));
      final ownerText = tester.widget<Text>(
        find.byWidgetPredicate(
          (widget) =>
              widget is Text && widget.textSpan?.toPlainText() == 'You · Owner',
        ),
      );
      final ownerSpans = (ownerText.textSpan! as TextSpan).children!;
      expect(
        ownerSpans.last.style?.fontSize,
        AppTheme.light().textTheme.bodySmall?.fontSize,
      );
      expect(
        tester
            .widget<AvatarImage>(
              find.descendant(of: aliceRow, matching: find.byType(AvatarImage)),
            )
            .radius,
        20,
      );
      expect(
        find.descendant(
          of: aliceRow,
          matching: find.byIcon(BuzzIcons.chevronRight),
        ),
        findsOneWidget,
      );
      expect(tester.getSize(aliceRow).height, 40 + (Grid.xxs * 2));

      await Scrollable.ensureVisible(tester.element(aliceRow), alignment: .5);
      await tester.pumpAndSettle();
      await tester.tap(aliceRow);
      await tester.pumpAndSettle();
      expect(find.byType(UserProfileSheet), findsOneWidget);
    },
  );
}
