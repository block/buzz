part of '../channel_detail_page_test.dart';

void _savedChannelPreviewTests() {
  final cached = _textMsg(
    id: 'cached-preview',
    pubkey: 'viewer',
    content: 'Saved preview before reconnect',
  );
  final fresh = _textMsg(
    id: 'fresh-relay-page',
    pubkey: 'viewer',
    content: 'Fresh history after reconnect',
    createdAt: 1001,
  );
  final outgoing = _textMsg(
    id: 'outgoing-message',
    pubkey: 'viewer',
    content: 'Message sent while history is pending',
    createdAt: 1002,
  );
  final preview = (
    events: [cached],
    summaries: <String, ChannelWindowThreadSummary>{},
  );

  testWidgets(
    'a saved channel shows live updates and sends until relay history arrives',
    (tester) async {
      final relayMessages = _FakeMessagesNotifier([], hasLoadedMessages: false);
      await tester.pumpWidget(
        _buildTestable(
          messages: [],
          messagesNotifier: relayMessages,
          savedPreview: Future.value(preview),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text(cached.content), findsOneWidget);

      relayMessages.setMessages([cached, fresh], historyCompleted: false);
      relayMessages.addLocalMessage(outgoing);
      await tester.pumpAndSettle();
      expect(find.text(cached.content), findsOneWidget);
      expect(find.text(fresh.content), findsOneWidget);
      expect(find.text(outgoing.content), findsOneWidget);
      expect(
        tester.getCenter(find.text(outgoing.content)).dy,
        greaterThan(tester.getCenter(find.text(cached.content)).dy),
      );

      relayMessages.setMessages([fresh, outgoing]);
      await tester.pumpAndSettle();
      expect(find.text(fresh.content), findsOneWidget);
      expect(find.text(outgoing.content), findsOneWidget);
      expect(find.text(cached.content), findsNothing);
    },
  );

  testWidgets('a late disk read cannot replace an empty relay result', (
    tester,
  ) async {
    final pending = Completer<SavedChannelPreview?>();
    final relayMessages = _FakeMessagesNotifier([], hasLoadedMessages: false);
    await tester.pumpWidget(
      _buildTestable(
        messages: [],
        messagesNotifier: relayMessages,
        savedPreview: pending.future,
      ),
    );
    await tester.pumpAndSettle();

    relayMessages.setMessages([]);
    pending.complete(preview);
    await tester.pumpAndSettle();
    expect(find.text(cached.content), findsNothing);
  });
}
