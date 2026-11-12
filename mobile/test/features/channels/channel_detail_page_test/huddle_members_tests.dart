part of '../channel_detail_page_test.dart';

void huddleMembersTests() {
  testWidgets('top action discovers a Huddle outside the timeline window', (
    tester,
  ) async {
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    await tester.pumpWidget(
      _buildTestable(
        messages: const [],
        huddleLifecycle: [
          _huddleMsg(
            id: 'off-window-huddle',
            kind: EventKind.huddleStarted,
            createdAt: now,
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.byTooltip('Open Huddle'), findsOneWidget);
    expect(find.text('Huddle in progress'), findsNothing);
  });
  testWidgets('native Solo header keeps settings and exposes Huddle', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    await tester.pumpWidget(
      _buildTestable(personalChat: true, messages: const []),
    );
    await tester.pumpAndSettle();
    final view = tester.widget<UiKitView>(
      find.byWidgetPredicate(
        (widget) =>
            widget is UiKitView && widget.viewType == 'buzz/ios_navigation_bar',
      ),
    );
    final payload = view.creationParams! as Map<String, Object?>;
    final actions = payload['actions']! as List;
    expect(actions.map((action) => action['label']), [
      'Chat settings',
      'Start Huddle',
    ]);
    expect(actions.last['enabled'], isTrue);
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('Solo starts a call and retries invitations only in its Huddle', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(400, 850);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final relay = _HuddleInvitationRelay();
    final transport = _HuddleTestTransport(
      peers: const {
        1: HuddlePeer(
          pubkey:
              '79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798',
          peerIndex: 1,
          epoch: 0,
        ),
      },
    );
    final solo = Channel(
      id: _channelId,
      name: 'Solo call',
      description: '',
      channelType: 'stream',
      labels: const ['buzz.solo'],
      visibility: 'private',
      createdBy:
          '79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798',
      createdAt: DateTime(2025),
      memberCount: 1,
      isMember: true,
    );
    await tester.pumpWidget(
      _buildTestable(
        channel: solo,
        personalChat: true,
        signingPubkey:
            '79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798',
        messages: const [],
        textScaler: const TextScaler.linear(1.3),
        disableAnimations: true,
        directoryPeople: const [
          UserProfile(
            pubkey:
                'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
            displayName: 'Guest',
          ),
          UserProfile(
            pubkey:
                'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
            displayName: 'Voice Agent',
            ownerPubkey:
                '79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798',
          ),
        ],
        knownAgentPubkeys: const {
          'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
        },
        users: const {
          'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa':
              UserProfile(
                pubkey:
                    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                displayName: 'Guest',
              ),
          'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb':
              UserProfile(
                pubkey:
                    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                displayName: 'Voice Agent',
                ownerPubkey:
                    '79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798',
              ),
        },
        relayConfigNotifier: _HuddleRelayConfigNotifier(),
        relaySessionNotifier: relay,
        huddleCurrentPubkey:
            '79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798',
        huddleMediaFactory: _HuddleTestMedia.new,
        huddleTransportFactory: (_) => transport,
      ),
    );
    await tester.pumpAndSettle();
    expect(find.byTooltip('Chat settings'), findsOneWidget);
    await tester.tap(find.byTooltip('Start Huddle'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Cancel'));
    await tester.pumpAndSettle();
    expect(relay.accepted, isEmpty);
    await tester.tap(find.byTooltip('Start Huddle'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, 'Start Huddle'));
    await tester.pumpAndSettle();
    final start = relay.accepted.singleWhere(
      (e) => e.kind == EventKind.huddleStarted,
    );
    final room = (jsonDecode(start.content) as Map)['ephemeral_channel_id'];
    expect(start.channelId, _channelId);
    expect(find.bySemanticsLabel('Add members'), findsOneWidget);

    Future<void> openPicker() async {
      await tester.tap(find.byKey(const ValueKey('huddle-add-members')));
      await tester.pumpAndSettle();
      expect(
        find.textContaining('not be added to the original chat'),
        findsOneWidget,
      );
      await tester.tap(find.text('Choose members'));
      await tester.pumpAndSettle();
    }

    Future<void> choose(String name) async {
      await tester.enterText(
        find.byKey(const ValueKey('add-channel-members-search')),
        name,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text(name).last);
      await tester.pumpAndSettle();
    }

    await openPicker();
    await choose('Guest');
    await choose('Voice Agent');
    await tester.tap(find.byKey(const ValueKey('add-channel-members-submit')));
    await tester.pumpAndSettle();
    expect(find.textContaining('Invitation refused'), findsOneWidget);
    final human = relay.accepted.singleWhere((e) => e.kind == 9000);
    expect(human.channelId, room);
    expect(human.tags, contains(equals(['role', 'member'])));
    expect(find.text('Add member'), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('add-channel-members-submit')));
    await tester.pumpAndSettle();
    final invites = relay.accepted.where((e) => e.kind == 9000).toList();
    expect(invites, hasLength(2));
    expect(invites.last.channelId, room);
    expect(invites.last.tags, contains(equals(['role', 'bot'])));
    expect(invites.every((e) => e.channelId != _channelId), isTrue);

    await openPicker();
    await choose('Guest');
    relay.roster = [
      NostrEvent(
        id: 'current-roster',
        pubkey: 'relay',
        createdAt: 1,
        kind: 39002,
        tags: [
          ['d', room as String],
          [
            'p',
            'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
            '',
            'owner',
          ],
        ],
        content: '',
        sig: '',
      ),
    ];
    await tester.tap(find.byKey(const ValueKey('add-channel-members-submit')));
    await tester.pumpAndSettle();
    expect(relay.accepted.where((e) => e.kind == 9000), hasLength(2));
    await openPicker();
    await choose('Voice Agent');
    final container = ProviderScope.containerOf(
      tester.element(find.byType(ChannelDetailPage, skipOffstage: false)),
    );
    await container.read(mobileHuddleControllerProvider.notifier).leave();
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('add-channel-members-submit')));
    await tester.pumpAndSettle();
    expect(find.textContaining('no longer connected'), findsOneWidget);
    expect(relay.accepted.where((e) => e.kind == 9000), hasLength(2));
    expect(tester.takeException(), isNull);
  });
}

class _HuddleInvitationRelay extends _VoiceDmRelaySession {
  final accepted = <NostrEvent>[];
  var rejectAgent = true;
  List<NostrEvent> roster = [];

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filter.kinds.contains(39002)) return roster;
    return super.fetchHistory(filter, timeout: timeout);
  }

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (event.kind == 9000 &&
        event.tags.any(
          (tag) =>
              tag.length > 1 &&
              tag[0] == 'p' &&
              tag[1] ==
                  'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
        ) &&
        rejectAgent) {
      rejectAgent = false;
      throw StateError('Invitation refused');
    }
    accepted.add(event);
    return event;
  }
}
