part of '../channel_detail_page_test.dart';

void messageWidthTests() {
  for (final thread in [false, true]) {
    for (final width in [390.0, 600.0]) {
      for (final scale in [1.0, 2.0]) {
        testWidgets('text bubble width thread=$thread width=$width scale=$scale', (
          tester,
        ) async {
          tester.view.physicalSize = Size(width, 1500);
          tester.view.devicePixelRatio = 1;
          addTearDown(tester.view.reset);
          final events = [
            _textMsg(
              id: threadIdRoot,
              pubkey: 'alice',
              content:
                  'We can meet at the cafe after lunch. There is plenty of room for everyone, and we can decide where to go from there.',
            ),
            _textMsg(
              id: threadIdR1,
              pubkey: 'self',
              content:
                  'That sounds good. I will bring the notes so we can go through the plans together before heading out.',
              createdAt: 1100,
              extraTags: thread
                  ? [
                      ['e', threadIdRoot, '', 'reply'],
                    ]
                  : [],
            ),
          ];
          if (scale == 2) {
            events.add(
              _reaction(
                id: 'alignment-reaction',
                targetId: threadIdRoot,
                pubkey: 'self',
              ),
            );
          }
          final messages = formatTimeline(events);
          await tester.pumpWidget(
            _buildTestable(
              signingPubkey: 'self',
              textScaler: TextScaler.linear(scale),
              users: const {
                'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
                'self': UserProfile(pubkey: 'self', displayName: 'Me'),
              },
              messages: events,
              home: thread
                  ? ThreadDetailPage(
                      threadHead: messages.first,
                      allMessages: messages,
                      channelId: _channelId,
                      currentPubkey: 'self',
                      isMember: true,
                      isArchived: false,
                    )
                  : null,
            ),
          );
          await tester.pumpAndSettle();
          final prefix = thread ? 'thread-message-row' : 'message-row';
          for (final id in [threadIdRoot, threadIdR1]) {
            final row = find.byKey(ValueKey('$prefix-$id'));
            final bubble = find
                .descendant(
                  of: row,
                  matching: find.byType(LocalSendBubbleSurface),
                )
                .first;
            final rect = tester.getRect(bubble);
            final avatar = find.descendant(
              of: row,
              matching: find.byType(MessageMediaAvatar),
            );
            if (avatar.evaluate().isNotEmpty) {
              expect(
                tester.getRect(avatar.first).bottom,
                closeTo(rect.bottom, 0.01),
              );
            }
            expect(rect.width, closeTo(width * 2 / 3, 1));
            final paragraph = tester.widget<RichText>(
              find
                  .descendant(of: bubble, matching: find.byType(RichText))
                  .first,
            );
            expect(paragraph.textAlign, TextAlign.start);
            expect(paragraph.textDirection, TextDirection.ltr);
            if (id == threadIdR1) {
              expect(rect.right, closeTo(tester.getRect(row).right, 0.01));
            }
          }
          expect(tester.takeException(), isNull);
        });
      }
    }
  }
}
