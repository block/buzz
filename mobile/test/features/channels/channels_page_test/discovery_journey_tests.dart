part of '../channels_page_test.dart';

void _discoveryJourneyTests(
  Widget Function({required List<Override> overrides}) buildTestable,
) {
  testWidgets(
    'zero-membership user retries discovery and joins a conversation',
    (tester) async {
      final session = _DiscoveryJourneySession();
      await tester.pumpWidget(
        buildTestable(
          overrides: [
            relaySessionProvider.overrideWith(() => session),
            myPubkeyProvider.overrideWith((ref) => 'self'),
            channelActionsProvider.overrideWith(
              (ref) => ChannelActions(
                ref: ref,
                session: session,
                signedEventRelay: _DiscoveryJourneyRelay(session),
                currentPubkey: 'self',
              ),
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();
      final container = ProviderScope.containerOf(
        tester.element(find.byType(ChannelsPage)),
      );
      expect(container.read(channelsProvider).requireValue, isEmpty);
      expect(session.directoryRequests, 0);

      await tester.tap(find.byKey(const Key('browse-channels-button')));
      await tester.pumpAndSettle();
      expect(find.text('Couldn’t load open channels.'), findsOneWidget);
      await tester.tap(find.byKey(const Key('browse-channels-retry')));
      await tester.pumpAndSettle();
      expect(find.text('community-help'), findsOneWidget);
      expect(find.text('private-planning'), findsNothing);
      expect(find.text('old-channel'), findsNothing);
      expect(find.text('hidden-dm'), findsNothing);
      expect(
        container.read(channelsProvider).requireValue.every((c) => !c.isMember),
        isTrue,
      );

      await tester.tap(
        find.byKey(const Key('browse-channel-join-$_discoveryId')),
      );
      await tester.pumpAndSettle();
      expect(session.joined, isTrue);
      expect(find.byType(BottomSheet), findsNothing);
      expect(find.text('community-help'), findsOneWidget);
      expect(
        container
            .read(channelsProvider)
            .requireValue
            .singleWhere((channel) => channel.id == _discoveryId)
            .isMember,
        isTrue,
      );
      // Discovery remains available after acquiring a membership.
      expect(find.byKey(const Key('browse-channels-button')), findsOneWidget);
    },
  );
}

const _discoveryId = '11111111-1111-4111-8111-111111111111';

class _DiscoveryJourneySession extends _ReconnectingRelaySession {
  _DiscoveryJourneySession() : super(initialStatus: SessionStatus.connected);

  bool joined = false;
  int directoryRequests = 0;

  NostrEvent metadata(
    String id,
    String name, {
    String type = 'stream',
    bool private = false,
    bool archived = false,
  }) => NostrEvent(
    id: 'meta-$id',
    pubkey: 'creator',
    createdAt: 1,
    kind: 39000,
    tags: [
      ['d', id],
      ['name', name],
      ['t', type],
      [private ? 'private' : 'public'],
      if (archived) ['archived', 'true'],
    ],
    content: '',
    sig: 'sig',
  );

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final result = <NostrEvent>[];
    for (final filter in filters) {
      if (filter.until != null) continue;
      if (filter.kinds.contains(39000) && !filter.tags.containsKey('#d')) {
        directoryRequests++;
        if (directoryRequests == 1) throw Exception('Directory unavailable');
        result.addAll([
          metadata(_discoveryId, 'community-help'),
          metadata(
            '22222222-2222-4222-8222-222222222222',
            'private-planning',
            private: true,
          ),
          metadata(
            '33333333-3333-4333-8333-333333333333',
            'old-channel',
            archived: true,
          ),
          metadata(
            '44444444-4444-4444-8444-444444444444',
            'hidden-dm',
            type: 'dm',
          ),
        ]);
      } else {
        result.addAll(await fetchHistory(filter));
      }
    }
    return result;
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filter.kinds.contains(39002) && joined) {
      return [
        NostrEvent(
          id: 'membership',
          pubkey: 'creator',
          createdAt: 2,
          kind: 39002,
          tags: [
            ['d', _discoveryId],
            ['p', 'self'],
          ],
          content: '',
          sig: 'sig',
        ),
      ];
    }
    if (filter.kinds.contains(39000) &&
        (filter.tags['#d'] ?? []).contains(_discoveryId)) {
      return [metadata(_discoveryId, 'community-help')];
    }
    return [];
  }
}

// Fake only the signed network boundary; the widget, ChannelsNotifier and
// ChannelActions.joinChannel/refresh are the production implementations.
class _DiscoveryJourneyRelay extends SignedEventRelay {
  _DiscoveryJourneyRelay(this.session) : super(session: session, nsec: null);
  final _DiscoveryJourneySession session;

  @override
  Future<NostrEvent> submit({
    required int kind,
    required String content,
    required List<List<String>> tags,
    int? createdAt,
    void Function(NostrEvent)? onSigned,
  }) async {
    expect(kind, 9021);
    expect(tags, [
      ['h', _discoveryId],
    ]);
    session.joined = true;
    return NostrEvent(
      id: 'join',
      pubkey: 'self',
      createdAt: 2,
      kind: kind,
      tags: tags,
      content: '',
      sig: 'sig',
    );
  }
}
