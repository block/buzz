part of '../channels_page_test.dart';

void _communityUnreadLandingTests(
  Widget Function({
    required List<Override> overrides,
    _FakeProfileNotifier? profile,
  })
  buildTestable,
) {
  testWidgets('unread deadline lands the destination without a 20-second wait', (
    tester,
  ) async {
    final communities = [
      for (final id in ['alpha', 'bravo'])
        Community(
          id: id,
          name: id,
          relayUrl: 'wss://$id.example.com',
          addedAt: DateTime(2025),
        ),
    ];
    final communitiesNotifier = _FakeCommunityListNotifier(communities);
    final session = _UnreadDeadlineLandingSession(communitiesNotifier);
    // The header avatar's own profile never arrives. It must not gate landing.
    final profile = _StalledProfileNotifier();
    await tester.pumpWidget(
      buildTestable(
        profile: profile,
        overrides: [
          channelSectionsProvider.overrideWith(_LandingSections.new),
          channelStarsProvider.overrideWith(_LandingStars.new),
          channelMutesProvider.overrideWith(_LandingMutes.new),
          channelSortProvider.overrideWith(_LandingSort.new),
          readStateProvider.overrideWith(
            () => _FakeReadStateNotifier(
              const ReadStateState(
                isReady: true,
                pubkey: 'pk',
                contexts: {},
                version: 0,
              ),
            ),
          ),
          myPubkeyProvider.overrideWith((ref) => 'pk'),
          relaySessionProvider.overrideWith(() => session),
          communityListProvider.overrideWith(() => communitiesNotifier),
          activeCommunityProvider.overrideWith((ref) async {
            await ref.watch(communityListProvider.future);
            return communities.firstWhere(
              (c) => c.id == communitiesNotifier.activeId,
            );
          }),
        ],
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(
      find.byWidgetPredicate(
        (widget) =>
            widget is Semantics &&
            widget.properties.label == 'Community settings',
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('community-menu-switch')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('community-switcher-row-bravo')));

    // Exercise the real ChannelsNotifier and actual picker. The whole landing
    // must finish in three seconds of animation frames, not the error deadline.
    for (var frame = 0; frame < 30; frame++) {
      await tester.pump(const Duration(milliseconds: 100));
    }
    expect(session.destinationUnreadRequests, greaterThan(0));
    expect(profile.stalled, isTrue);
    expect(find.byKey(const Key('community-switcher-page')), findsNothing);
    expect(find.byKey(const Key('community-switch-loading')), findsNothing);
    expect(find.text('bravo'), findsNothing);
    expect(find.text('bravo-channel'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });
}

class _UnreadDeadlineLandingSession extends RelaySessionNotifier {
  _UnreadDeadlineLandingSession(this.communities);
  final _FakeCommunityListNotifier communities;
  int destinationUnreadRequests = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  NostrEvent event(int kind, List<List<String>> tags) => NostrEvent(
    id: '$kind-${communities.activeId}',
    pubkey: 'pk',
    createdAt: 1,
    kind: kind,
    tags: tags,
    content: '',
    sig: 'sig',
  );

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filters.length == 1 && filters.single.kinds.contains(39002)) {
      return filters.single.until == null
          ? [
              event(39002, [
                ['d', '${communities.activeId}-channel'],
                ['p', 'pk'],
              ]),
            ]
          : [];
    }
    if (communities.activeId == 'bravo' &&
        filters.isNotEmpty &&
        filters.every((f) => f.since != null)) {
      destinationUnreadRequests++;
      throw RelayException(503, '{"error":"query timed out"}');
    }
    return [];
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => filter.kinds.contains(39000)
      ? [
          event(39000, [
            ['d', '${communities.activeId}-channel'],
            ['name', '${communities.activeId}-channel'],
            ['t', 'stream'],
            ['public'],
          ]),
        ]
      : [];

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String)? onClosed,
  }) async => () {};
}

class _LandingSections extends ChannelSectionsNotifier {
  @override
  ChannelSectionsState build() => const ChannelSectionsState(isReady: true);
}

class _LandingStars extends ChannelStarsNotifier {
  @override
  ChannelStarsState build() => const ChannelStarsState(isReady: true);
}

class _LandingMutes extends ChannelMutesNotifier {
  @override
  ChannelMutesState build() => const ChannelMutesState(isReady: true);
}

class _LandingSort extends ChannelSortNotifier {
  @override
  ChannelSortState build() => const ChannelSortState(isReady: true);
}

class _StalledProfileNotifier extends _FakeProfileNotifier {
  bool stalled = false;
  int _builds = 0;

  @override
  Future<UserProfile?> build() {
    // Rebuild on the community switch, as the real profile does per relay.
    ref.watch(activeCommunityProvider);
    // Origin Home loads normally; every later rebuild hangs forever.
    if (_builds++ == 0) return super.build();
    stalled = true;
    return Completer<UserProfile?>().future;
  }
}

void _communityCachedLandingTests(
  Widget Function({required List<Override> overrides}) buildTestable,
) {
  testWidgets('a cached destination lands while its relay never answers', (
    tester,
  ) async {
    final communities = [
      for (final id in ['alpha', 'bravo'])
        Community(
          id: id,
          name: id,
          relayUrl: 'wss://$id.example.com',
          addedAt: DateTime(2025),
        ),
    ];
    final cached = Channel(
      id: 'bravo-cached',
      name: 'bravo-cached-channel',
      channelType: 'stream',
      visibility: 'open',
      description: '',
      createdBy: 'pk',
      createdAt: DateTime(2025),
      memberCount: 1,
      isMember: true,
    );
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final storage = ChannelListSnapshotStorage(prefs);
    expect(
      await storage.write(
        storage.capture('pk', 'wss://bravo.example.com'),
        ChannelListSnapshot(channels: [cached.toJson()], hiddenDmIds: {}),
      ),
      isTrue,
    );
    final communitiesNotifier = _FakeCommunityListNotifier(communities);
    final session = _StalledDestinationSession(communitiesNotifier);
    await tester.pumpWidget(
      buildTestable(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          // Remote preference state never becomes ready for the destination.
          channelSectionsProvider.overrideWith(_UnreadyLandingSections.new),
          channelStarsProvider.overrideWith(_LandingStars.new),
          channelMutesProvider.overrideWith(_LandingMutes.new),
          channelSortProvider.overrideWith(_LandingSort.new),
          readStateProvider.overrideWith(
            () => _FakeReadStateNotifier(
              const ReadStateState(
                isReady: true,
                pubkey: 'pk',
                contexts: {},
                version: 0,
              ),
            ),
          ),
          myPubkeyProvider.overrideWith((ref) => 'pk'),
          relaySessionProvider.overrideWith(() => session),
          communityListProvider.overrideWith(() => communitiesNotifier),
          activeCommunityProvider.overrideWith((ref) async {
            await ref.watch(communityListProvider.future);
            return communities.firstWhere(
              (c) => c.id == communitiesNotifier.activeId,
            );
          }),
        ],
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(
      find.byWidgetPredicate(
        (widget) =>
            widget is Semantics &&
            widget.properties.label == 'Community settings',
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('community-menu-switch')));
    await tester.pumpAndSettle();
    session.stallDestination = true;
    await tester.tap(find.byKey(const Key('community-switcher-row-bravo')));

    // Only animation frames: the relay never answers, nothing hits a deadline.
    for (var frame = 0; frame < 20; frame++) {
      await tester.pump(const Duration(milliseconds: 100));
    }
    expect(session.stalledRequests, greaterThan(0));
    expect(find.byKey(const Key('community-switcher-page')), findsNothing);
    expect(find.byKey(const Key('community-switch-loading')), findsNothing);
    expect(find.text('bravo-cached-channel'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });
}

/// Answers the origin normally; destination requests never complete.
class _StalledDestinationSession extends _UnreadDeadlineLandingSession {
  _StalledDestinationSession(super.communities);
  bool stallDestination = false;
  int stalledRequests = 0;

  Future<T>? _stall<T>() {
    if (!stallDestination || communities.activeId != 'bravo') return null;
    stalledRequests++;
    return Completer<T>().future;
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) => _stall() ?? super.queryRelay(filters, timeout: timeout);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) => _stall() ?? super.fetchHistory(filter, timeout: timeout);
}

class _UnreadyLandingSections extends ChannelSectionsNotifier {
  @override
  ChannelSectionsState build() => const ChannelSectionsState(isReady: false);
}
