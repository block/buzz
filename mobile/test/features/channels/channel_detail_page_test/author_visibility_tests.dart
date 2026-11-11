part of '../channel_detail_page_test.dart';

void messageAuthorVisibilityTests() {
  for (final dark in [false, true]) {
    for (final kind in [
      'dm',
      'solo',
      'group',
      'channel',
      'unknown',
      'outsider',
    ]) {
      testWidgets('message identity: $kind dark=$dark', (tester) async {
        final participants = kind == 'unknown'
            ? <String>[]
            : ['self', 'alice', if (kind == 'group') 'bob'];
        final channel = Channel(
          id: _channelId,
          name: 'Conversation',
          channelType: kind == 'channel' || kind == 'solo' ? 'stream' : 'dm',
          labels: kind == 'solo' ? const ['buzz.solo'] : const [],
          visibility: 'private',
          description: '',
          createdBy: 'self',
          createdAt: DateTime(2025),
          memberCount: participants.length,
          participantPubkeys: participants,
          isMember: true,
        );
        await tester.pumpWidget(
          _buildTestable(
            channel: channel,
            personalChat: kind == 'solo',
            darkTheme: dark,
            signingPubkey: 'self',
            members: [
              for (final pubkey in participants)
                ChannelMember(
                  pubkey: pubkey,
                  role: 'member',
                  joinedAt: DateTime(2025),
                ),
            ],
            users: const {
              'self': UserProfile(pubkey: 'self', displayName: 'Me'),
              'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
              'bob': UserProfile(pubkey: 'bob', displayName: 'Bob'),
            },
            messages: [
              _textMsg(
                id: 'incoming',
                pubkey: kind == 'outsider' ? 'bob' : 'alice',
                content: 'Hello',
              ),
              _textMsg(
                id: 'outgoing',
                pubkey: 'self',
                content: 'Hi',
                createdAt: 1100,
              ),
            ],
          ),
        );
        await tester.pumpAndSettle();
        final name = find.byKey(const ValueKey('message-author-incoming'));
        final hidesName = kind == 'dm' || kind == 'solo';
        expect(name, hidesName ? findsNothing : findsOneWidget);
        final incomingRow = find.byKey(const ValueKey('message-row-incoming'));
        final outgoingRow = find.byKey(const ValueKey('message-row-outgoing'));
        final incomingAvatars = find.descendant(
          of: incomingRow,
          matching: find.byType(AvatarImage),
        );
        expect(incomingAvatars, hidesName ? findsNothing : findsOneWidget);
        expect(
          find.descendant(of: outgoingRow, matching: find.byType(AvatarImage)),
          findsNothing,
        );

        expect(
          find.byKey(const ValueKey('message-timestamp-incoming')),
          findsOneWidget,
        );
        expect(
          find.byKey(const ValueKey('message-timestamp-outgoing')),
          findsOneWidget,
        );
        final ownName = find.byKey(const ValueKey('message-author-outgoing'));
        final hidesOwnName = hidesName || kind == 'outsider';
        expect(ownName, hidesOwnName ? findsNothing : findsOneWidget);
        if (!hidesOwnName) {
          expect(tester.widget<Text>(ownName).data, 'You');
        }
        if (!hidesName) {
          final context = tester.element(name);
          expect(
            tester.widget<Text>(name).style?.color,
            context.tertiaryTextColor,
          );
        }
        final root = _textMsg(
          id: 'incoming',
          pubkey: kind == 'outsider' ? 'bob' : 'alice',
          content: 'Hello',
        );
        unawaited(
          Navigator.of(tester.element(incomingRow)).push(
            MaterialPageRoute<void>(
              builder: (_) => ThreadDetailPage(
                threadHead: formatTimeline([root]).single,
                allMessages: formatTimeline([root]),
                channelId: _channelId,
                currentPubkey: 'self',
                isMember: true,
                isArchived: false,
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();
        final threadRow = find.byKey(
          const ValueKey('thread-message-row-incoming'),
        );
        expect(threadRow, findsOneWidget);
        expect(
          find.descendant(of: threadRow, matching: find.byType(AvatarImage)),
          hidesName ? findsNothing : findsOneWidget,
        );
        expect(tester.takeException(), isNull);
      });
    }
  }
}
