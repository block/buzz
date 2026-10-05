part of '../channel_detail_page_test.dart';

void _loadingReviewTests() {
  testWidgets('loaded page rebuild formats the timeline once', (tester) async {
    var transforms = 0;
    debugOnFormatTimeline = () => transforms++;
    addTearDown(() => debugOnFormatTimeline = null);
    await tester.pumpWidget(
      _buildTestable(
        messages: [
          _textMsg(id: 'one', pubkey: 'alice', content: 'Loaded message'),
        ],
      ),
    );
    await tester.pumpAndSettle();
    for (var i = 0; i < 3; i++) {
      transforms = 0;
      tester.element(find.byType(ChannelDetailPage)).markNeedsBuild();
      await tester.pump();
      expect(transforms, 1);
    }
  });

  testWidgets('reconnect snapshot puts newest cached media at the bottom', (
    tester,
  ) async {
    final relay = _ReconnectingRelaySession(
      initialStatus: SessionStatus.connected,
    );
    await tester.pumpWidget(
      _buildTestable(
        messages: [
          for (var i = 0; i < 2; i++)
            _textMsg(
              id: 'image-$i',
              pubkey: 'alice',
              createdAt: 1000 + i,
              content: '![photo](https://example.com/$i.jpg)',
              extraTags: [
                [
                  'imeta',
                  'url https://example.com/$i.jpg',
                  'm image/jpeg',
                  'dim 400x100',
                ],
              ],
            ),
        ],
        relaySessionNotifier: relay,
      ),
    );
    await tester.pumpAndSettle();
    final olderPreview = find.byKey(
      const ValueKey('message-media-image-preview:https://example.com/0.jpg'),
    );
    final newerPreview = find.byKey(
      const ValueKey('message-media-image-preview:https://example.com/1.jpg'),
    );
    expect(
      tester.getCenter(olderPreview).dy,
      lessThan(tester.getCenter(newerPreview).dy),
    );
    relay.setReconnecting();
    await tester.pump();
    await tester.pump(const Duration(seconds: 3));
    final olderShape = find.byKey(
      const ValueKey('message-skeleton-image:https://example.com/0.jpg'),
    );
    final newerShape = find.byKey(
      const ValueKey('message-skeleton-image:https://example.com/1.jpg'),
    );
    expect(
      tester.getCenter(olderShape).dy,
      lessThan(tester.getCenter(newerShape).dy),
    );
    final list = find
        .ancestor(of: newerShape, matching: find.byType(ListView))
        .first;
    expect(tester.widget<ListView>(list).reverse, isTrue);
    expect(
      tester.getCenter(newerShape).dy,
      greaterThan(tester.getSize(find.byType(Scaffold).first).height / 2),
    );
    relay.connect();
    await tester.pumpAndSettle();
    expect(
      tester.getCenter(olderPreview).dy,
      lessThan(tester.getCenter(newerPreview).dy),
    );
  });

  testWidgets(
    'same loading cycle updates connection semantics without changing shapes',
    (tester) async {
      final semantics = tester.ensureSemantics();
      final relay = _ReconnectingRelaySession(
        initialStatus: SessionStatus.connecting,
      );
      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          messagesNotifier: _FakeMessagesNotifier(
            const [],
            hasLoadedMessages: false,
          ),
          relaySessionNotifier: relay,
        ),
      );
      await tester.pump();
      final key = find.byKey(const Key('channel-detail-connection-skeleton'));
      expect(tester.getSemantics(key).label, 'Connecting');
      final reveal = tester.widget<SkeletonReveal>(find.byType(SkeletonReveal));
      expect(reveal.loading, isTrue);
      relay.setReconnecting();
      await tester.pump();
      expect(
        tester.widget<SkeletonReveal>(find.byType(SkeletonReveal)).loading,
        isTrue,
      );
      expect(tester.getSemantics(key).label, 'Reconnecting');
      semantics.dispose();
    },
  );
}
