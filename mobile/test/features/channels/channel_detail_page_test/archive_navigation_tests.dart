part of '../channel_detail_page_test.dart';

void archiveNavigationTests() {
  testWidgets('archive leaves a profile opened while it is pending', (
    tester,
  ) async {
    final archive = Completer<void>();
    await tester.pumpWidget(
      _buildTestable(
        messages: const [],
        huddleCurrentPubkey: 'self',
        users: const {'peer': UserProfile(pubkey: 'peer', displayName: 'Peer')},
        members: [
          ChannelMember(
            pubkey: 'self',
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
          ChannelMember(
            pubkey: 'peer',
            role: 'member',
            joinedAt: DateTime(2025),
          ),
        ],
        createChannelActions: (ref) =>
            _FakeChannelActions(ref, onArchiveChannel: (_) => archive.future),
        home: Builder(
          builder: (context) => Scaffold(
            body: TextButton(
              onPressed: () => Navigator.of(context).push<void>(
                MaterialPageRoute<void>(
                  builder: (_) => ChannelDetailPage(channel: _testChannel),
                ),
              ),
              child: const Text('Open conversation'),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Open conversation'));
    await tester.pumpAndSettle();
    await tester.tap(
      find.byKey(const ValueKey('channel-header-settings-trigger')),
    );
    await tester.pumpAndSettle();
    await tester.scrollUntilVisible(
      find.text('Archive channel').hitTestable(),
      200,
      scrollable: find.descendant(
        of: find.byKey(const ValueKey('channel-details-page-list')),
        matching: find.byType(Scrollable),
      ),
    );
    await tester.tap(find.text('Archive channel'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Archive'));
    await tester.pumpAndSettle();

    await tester.scrollUntilVisible(
      find.byKey(const ValueKey('channel-details-member-peer')).hitTestable(),
      -200,
      scrollable: find.descendant(
        of: find.byKey(const ValueKey('channel-details-page-list')),
        matching: find.byType(Scrollable),
      ),
    );
    await tester.tap(find.byKey(const ValueKey('channel-details-member-peer')));
    await tester.pumpAndSettle();
    expect(find.byType(UserProfileSheet), findsOneWidget);
    final profileRoute = ModalRoute.of(
      tester.element(find.byType(UserProfileSheet)),
    )!;
    expect(profileRoute.isCurrent, isTrue);

    archive.complete();
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    expect(find.byType(UserProfileSheet), findsOneWidget);
    expect(profileRoute.isActive, isTrue);
    expect(profileRoute.isCurrent, isTrue);
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pumpAndSettle();
  });
}
