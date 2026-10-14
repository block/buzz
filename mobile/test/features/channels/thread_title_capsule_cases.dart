part of 'channel_detail_page_test.dart';

void threadTitleCapsuleTests() {
  for (final dm in [false, true]) {
    testWidgets('iOS thread capsule uses live display name dm=$dm', (
      tester,
    ) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      final channel = Channel(
        id: _testChannel.id,
        visibility: 'open',
        description: '',
        createdBy: 'self',
        createdAt: _testChannel.createdAt,
        memberCount: 2,
        isMember: true,
        name: dm ? 'DM' : 'engineering',
        channelType: dm ? 'dm' : 'stream',
        participantPubkeys: dm ? ['self', 'alice'] : [],
        participants: dm ? ['Me', 'Alice'] : [],
      );
      final channels = _FakeChannelsNotifier([channel]);
      final root = _textMsg(
        id: 'capsule-root',
        pubkey: 'alice',
        content: 'Root',
      );
      final timeline = formatTimeline([root]);
      await tester.pumpWidget(
        _buildTestable(
          messages: [root],
          channel: channel,
          channelsNotifier: channels,
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
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
      Map params() =>
          tester
                  .widget<UiKitView>(
                    find.byWidgetPredicate(
                      (widget) =>
                          widget is UiKitView &&
                          widget.viewType == 'buzz/ios_navigation_bar',
                    ),
                  )
                  .creationParams!
              as Map;
      expect(params()['title'], 'Thread');
      expect(params()['subtitle'], dm ? 'Alice' : 'engineering');
      expect(params()['titleEnabled'], false);
      channels.setChannels([channel.copyWith(name: 'Renamed conversation')]);
      await tester.pumpAndSettle();
      expect(params()['subtitle'], 'Renamed conversation');
      expect(params()['titleEnabled'], false);
      channels.setChannels([]);
      await tester.pumpAndSettle();
      // An empty subtitle keeps the shared capsule while metadata is absent.
      expect(params()['subtitle'], '');
      expect(tester.takeException(), isNull);
      debugDefaultTargetPlatformOverride = null;
    });
  }
}
