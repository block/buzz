part of 'channels_provider_test.dart';

void _requestPathCases() {
  test(
    'keeps joined channels beyond the relay cap with archived rooms',
    () async {
      final ids = List.generate(
        1080,
        (index) =>
            '${index.toString().padLeft(8, '0')}-0000-4000-8000-000000000000',
      );
      final session = _CappedChannelSession(
        memberships: [
          for (var i = 0; i < ids.length; i++)
            _membership(
              ids[i],
              'me',
              additionalPubkey: 'alice',
              eventId: (i + 2000).toRadixString(16).padLeft(64, '0'),
            ),
        ],
        metadata: [
          for (var i = 0; i < ids.length; i++)
            _cappedMeta(ids[i], i, archived: i < 1016),
          // Old addressable revisions may coexist after a relay backfill. They
          // must not crowd another channel out of even a bounded ID chunk.
          for (var i = 0; i < 501; i++)
            _cappedMeta(ids[1000], i + 3000, archived: true, createdAt: 15),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      final channels = await container.read(channelsProvider.future);
      expect(channels, hasLength(1080));
      expect(
        channels
            .where((channel) => !channel.isArchived)
            .map((channel) => channel.id),
        unorderedEquals(ids.skip(1016)),
      );
      expect(channels.every((channel) => channel.memberCount == 2), isTrue);
      expect(
        session.historyFilters
            .where((filter) => filter.kinds.contains(39000))
            .every(
              (filter) =>
                  filter.tags['#d']!.length <= 500 && filter.limit <= 500,
            ),
        isTrue,
      );
      expect(
        container
            .read(channelsProvider.notifier)
            .cachedMembersForChannel(ids.last)
            .map((member) => member.pubkey),
        ['me', 'alice'],
      );

      // A failed later page cannot replace the complete cached list with the
      // prefix that happened to arrive. The same refresh must be retryable.
      session.failMetadataRequest = session.metadataRequests + 2;
      await container.read(channelsProvider.notifier).refresh();
      expect(session.metadataRequests, session.failMetadataRequest);
      expect(container.read(channelsProvider).requireValue, hasLength(1080));
      session.failMetadataRequest = null;
      await container.read(channelsProvider.notifier).refresh();
      expect(container.read(channelsProvider).requireValue, hasLength(1080));

      session.ignoreMetadataCursor = true;
      await expectLater(
        container.read(channelsProvider.notifier).fetchFresh(),
        throwsStateError,
      );
      expect(container.read(channelsProvider).requireValue, hasLength(1080));
      session.ignoreMetadataCursor = false;
      await container.read(channelsProvider.notifier).refresh();
      expect(container.read(channelsProvider).requireValue, hasLength(1080));
    },
  );

  test('DM hydration overlaps history and shares admitted profiles', () async {
    final session = _RequestPathSession();
    final profilesGate = session.profilesGate = Completer<void>();
    final latestGate = session.latestGate = Completer<void>();
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);
    final load = container.read(channelsProvider.future);

    await _settle();
    expect(session.profileRequests, 1);
    expect(session.latestRequests, 1);
    expect(container.read(channelsProvider).hasValue, isFalse);
    final cacheReady = Completer<void>();
    final cacheSubscription = container.listen(userCacheProvider, (_, next) {
      if (next.containsKey('alice') && !cacheReady.isCompleted) {
        cacheReady.complete();
      }
    });
    addTearDown(cacheSubscription.close);
    profilesGate.complete();
    await cacheReady.future;
    expect(container.read(channelsProvider).hasValue, isFalse);
    latestGate.complete();

    final channel = (await load).single;
    expect(channel.displayLabel(currentPubkey: 'me'), 'Alice');
    expect(channel.memberCount, 2);
    expect(channel.lastMessageAt?.millisecondsSinceEpoch, 50000);
    expect(
      container.read(userCacheProvider)['alice']?.avatarUrl,
      'https://example.com/alice.png',
    );
    expect(
      await container.read(userCacheProvider.notifier).preload(['alice']),
      isTrue,
    );
    expect(session.profileRequests, 1);

    // Full refreshes discover relay-side profile changes without another
    // surface injecting a live cache event. Blank names retain the fallback.
    session.profiles = [
      _requestProfile(
        'alice',
        name: ' ',
        handle: ' alice@example.com ',
        createdAt: 2,
      ),
    ];
    // Ordinary reconnects stay metadata-only and reuse the admitted profile.
    session.setStatus(SessionStatus.disconnected);
    await _settle();
    session.setStatus(SessionStatus.connected);
    await _settle();
    expect(session.profileRequests, 1);
    expect(
      container
          .read(channelsProvider)
          .requireValue
          .single
          .displayLabel(currentPubkey: 'me'),
      'Alice',
    );
    await container.read(channelsProvider.notifier).refresh();
    expect(
      container
          .read(channelsProvider)
          .requireValue
          .single
          .displayLabel(currentPubkey: 'me'),
      'alice@example.com',
    );
    expect(session.profileRequests, 2);
    expect(
      container.read(userCacheProvider)['alice']?.nip05Handle?.trim(),
      'alice@example.com',
    );
    session.profiles = [
      _requestProfile('alice', name: ' ', handle: ' ', createdAt: 3),
    ];
    await container.read(channelsProvider.notifier).refresh();
    expect(
      container
          .read(channelsProvider)
          .requireValue
          .single
          .displayLabel(currentPubkey: 'me'),
      'Unknown identity',
    );
    expect(session.profileRequests, 3);
  });

  test(
    'late DM profiles cannot enter the destination cache or channel list',
    () async {
      final session = _RequestPathSession();
      final profilesGate = session.profilesGate = Completer<void>();
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      final oldLoad = container.read(channelsProvider.future);
      await _settle();
      expect(session.profileRequests, 1);

      session.profiles = [_requestProfile('alice', name: 'Destination Alice')];
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://destination.example');
      final destination = await container.read(channelsProvider.future);
      expect(
        destination.single.displayLabel(currentPubkey: 'me'),
        'Destination Alice',
      );
      profilesGate.complete();
      await oldLoad;
      await _settle();
      expect(
        container.read(userCacheProvider)['alice']?.displayName,
        'Destination Alice',
      );
      expect(
        container
            .read(channelsProvider)
            .requireValue
            .single
            .displayLabel(currentPubkey: 'me'),
        'Destination Alice',
      );
    },
  );

  test(
    'DM profile failures propagate while concurrent history is pending',
    () async {
      final failure = StateError('profile request failed');
      final session = _RequestPathSession()..profileFailure = failure;
      final latestGate = session.latestGate = Completer<void>();
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await expectLater(
        container.read(channelsProvider.future),
        throwsA(same(failure)),
      );
      expect(session.latestRequests, 1);
      latestGate.complete();
      await _settle();
      expect(container.read(channelsProvider).error, same(failure));
    },
  );

  test(
    'latest discovered roster owns Huddle filtering and member counts',
    () async {
      final session = _FakeRelaySession(
        memberships: [
          _membership(
            _channelA,
            'me',
            additionalPubkey: 'alice',
            createdAt: 2,
            eventId: 'new-a',
          ),
          _membership(
            _channelB,
            'me',
            ownerPubkey: 'me',
            createdAt: 2,
            eventId: 'new-b',
          ),
          _membership(_channelA, 'me', eventId: 'old-a'),
          _membership(
            _channelB,
            'me',
            ownerPubkey: 'imposter',
            eventId: 'old-b',
          ),
        ],
        metadata: [
          _meta(id: _channelA, name: 'General'),
          _meta(id: _channelB, name: 'Huddle', visibility: 'private'),
        ],
        huddleStarts: [
          _huddleStart(pubkey: 'me', parent: _channelA, backing: _channelB),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      final channels = await container.read(channelsProvider.future);
      expect(channels.single.id, _channelA);
      expect(channels.single.memberCount, 2);
      expect(
        container
            .read(channelsProvider.notifier)
            .cachedMembersForChannel(_channelA)
            .map((member) => member.pubkey),
        ['me', 'alice'],
      );
    },
  );
}

NostrEvent _cappedMeta(
  String channelId,
  int eventIndex, {
  required bool archived,
  int? createdAt,
}) => NostrEvent(
  id: eventIndex.toRadixString(16).padLeft(64, '0'),
  pubkey: 'creator',
  createdAt: createdAt ?? (archived ? 20 : 10),
  kind: 39000,
  tags: [
    ['d', channelId],
    ['name', 'channel-$channelId'],
    ['t', 'stream'],
    ['public'],
    if (archived) ['archived', 'true'],
  ],
  content: '',
  sig: 'sig',
);

// Protocol double: enforces the relay's per-filter 1000-event clamp and
// created_at DESC/id ASC composite cursor, including tied timestamps. Network
// authentication/signature checks remain outside this provider regression.
class _CappedChannelSession extends _FakeRelaySession {
  _CappedChannelSession({required super.memberships, required super.metadata});

  int metadataRequests = 0;
  int? failMetadataRequest;
  bool ignoreMetadataCursor = false;

  List<NostrEvent> _page(
    List<NostrEvent> events,
    NostrFilter filter, {
    bool ignoreBeforeId = false,
  }) {
    final beforeId = ignoreBeforeId
        ? null
        : filter.extensions['before_id'] as String?;
    final page =
        events.where((event) {
          final until = filter.until;
          return until == null ||
              event.createdAt < until ||
              (event.createdAt == until &&
                  (beforeId == null || event.id.compareTo(beforeId) > 0));
        }).toList()..sort((a, b) {
          final byTime = b.createdAt.compareTo(a.createdAt);
          return byTime != 0 ? byTime : a.id.compareTo(b.id);
        });
    return page.take(min(filter.limit, 1000)).toList();
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final query = ignoreMetadataCursor && filter.kinds.contains(39000)
        ? NostrFilter(
            kinds: filter.kinds,
            authors: filter.authors,
            ids: filter.ids,
            limit: filter.limit,
            since: filter.since,
            until: filter.until,
            search: filter.search,
            tags: filter.tags,
            extensions: {
              for (final entry in filter.extensions.entries)
                if (entry.key != 'before_id') entry.key: entry.value,
            },
          )
        : filter;
    final events = await super.fetchHistory(query, timeout: timeout);
    if (filter.kinds.contains(39000) && filter.tags.containsKey('#d')) {
      metadataRequests++;
      if (metadataRequests == failMetadataRequest) {
        throw StateError('joined metadata page failed');
      }
      return _page(events, filter, ignoreBeforeId: ignoreMetadataCursor);
    }
    return events;
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filters case [final filter]
        when filter.kinds.length == 1 &&
            filter.kinds.single == 39002 &&
            filter.tags.containsKey('#p')) {
      membershipQueryFilters.add(filter);
      return _page(memberships, filter);
    }
    return super.queryRelay(filters, timeout: timeout);
  }
}

NostrEvent _requestProfile(
  String pubkey, {
  required String name,
  String? handle,
  int createdAt = 1,
}) => NostrEvent(
  id: 'profile-$pubkey-$createdAt',
  pubkey: pubkey,
  createdAt: createdAt,
  kind: 0,
  tags: const [],
  content: jsonEncode({
    'display_name': name,
    'nip05': handle,
    'picture': 'https://example.com/$pubkey.png',
  }),
  sig: 'sig',
);

class _RequestPathSession extends _FakeRelaySession {
  _RequestPathSession()
    : super(
        memberships: [_membership(_channelA, 'me', additionalPubkey: 'alice')],
        metadata: [
          _meta(
            id: _channelA,
            name: 'DM',
            channelType: 'dm',
            participants: ['me', 'alice'],
          ),
        ],
        recentMessages: const [
          NostrEvent(
            id: 'dm-latest',
            pubkey: 'alice',
            createdAt: 50,
            kind: 9,
            tags: [
              ['h', _channelA],
            ],
            content: 'Hello',
            sig: 'sig',
          ),
        ],
      );

  List<NostrEvent> profiles = [_requestProfile('alice', name: 'Alice')];
  Completer<void>? profilesGate;
  Completer<void>? latestGate;
  Object? profileFailure;
  int profileRequests = 0;
  int latestRequests = 0;

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (!filter.kinds.contains(0)) {
      return super.fetchHistory(filter, timeout: timeout);
    }
    profileRequests++;
    final snapshot = List<NostrEvent>.of(profiles);
    final gate = profilesGate;
    profilesGate = null;
    await gate?.future;
    if (profileFailure case final failure?) throw failure;
    return snapshot;
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filters.isNotEmpty &&
        filters.every(
          (filter) => filter.kinds.contains(9) && filter.since == null,
        )) {
      latestRequests++;
      final gate = latestGate;
      latestGate = null;
      await gate?.future;
    }
    return super.queryRelay(filters, timeout: timeout);
  }
}
