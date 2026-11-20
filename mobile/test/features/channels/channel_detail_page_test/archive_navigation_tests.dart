part of '../channel_detail_page_test.dart';

void archiveNavigationTests() {
  testWidgets('archive leaves a profile opened while it is pending', (
    tester,
  ) async {
    final archive = Completer<void>();
    final dm = Channel(
      id: 'dm-peer',
      name: 'Peer',
      channelType: 'dm',
      visibility: 'private',
      description: '',
      createdBy: 'self',
      createdAt: DateTime(2025),
      memberCount: 2,
      isMember: true,
    );
    var dmOpened = false;
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
            _ArchiveDmActions(ref, archive, dm, () => dmOpened = true),
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
    // Settings stays open beneath the profile, as before this change.
    expect(
      find.byType(ChannelDetailsPage, skipOffstage: false),
      findsOneWidget,
    );

    // The profile still works: Message opens the DM.
    await tester.ensureVisible(find.text('Message'));
    await tester.tap(find.text('Message'));
    await tester.pumpAndSettle();
    expect(dmOpened, isTrue);
    expect(find.byType(UserProfileSheet), findsNothing);
    expect(
      tester
          .widget<ChannelDetailPage>(find.byType(ChannelDetailPage))
          .channel
          .id,
      dm.id,
    );
    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pumpAndSettle();
  });
}

class _ArchiveDmActions extends _FakeChannelActions {
  _ArchiveDmActions(super.ref, Completer<void> archive, this.dm, this.onOpened)
    : super(onArchiveChannel: (_) => archive.future);

  final Channel dm;
  final VoidCallback onOpened;

  @override
  Future<Channel> openDm({
    required List<String> pubkeys,
    bool waitForChannelList = true,
  }) async {
    onOpened();
    return dm;
  }
}
