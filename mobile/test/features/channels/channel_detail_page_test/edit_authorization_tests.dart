part of '../channel_detail_page_test.dart';

void _editAuthorizationTests() {
  testWidgets('chat applies owner edits only after verified owners load', (
    tester,
  ) async {
    final owner = nostr.Keys.generate();
    final agent = nostr.Keys.generate();
    final other = nostr.Keys.generate();
    final original = _textMsg(
      id: threadIdA,
      pubkey: agent.public,
      content: 'Original agent message',
    );
    NostrEvent edit(String signer, String content, int time) => NostrEvent(
      id: 'edit-$time',
      pubkey: signer,
      createdAt: time,
      kind: EventKind.streamMessageEdit,
      tags: [
        ['h', _channelId],
        ['e', original.id],
      ],
      content: content,
      sig: '',
    );
    final ownMessage = _textMsg(
      id: threadIdRoot,
      pubkey: owner.public,
      content: 'Original author message',
    );
    final ownEdit = NostrEvent(
      id: 'author-edit',
      pubkey: owner.public,
      createdAt: 2000,
      kind: EventKind.streamMessageEdit,
      tags: [
        ['h', _channelId],
        ['e', ownMessage.id],
      ],
      content: 'Author update',
      sig: '',
    );
    final profiles = Completer<List<NostrEvent>>();
    final session = _EditRelaySession(agent.public, profiles.future, [
      original,
      ownMessage,
      ownEdit,
      edit(owner.public, 'Owner update', 2001),
    ]);
    await tester.pumpWidget(
      _buildTestable(
        messages: const [],
        useRealEditProviders: true,
        relaySessionNotifier: session,
        channel: _testChannel.copyWith(isMember: false),
      ),
    );
    await tester.pumpAndSettle();
    final context = tester.element(find.byType(ChannelDetailPage));
    final container = ProviderScope.containerOf(context);
    expect(container.read(agentOwnersProvider).isLoading, isTrue);
    expect(findRichText('Original agent message'), findsOneWidget);
    expect(findRichText('Author update (edited)'), findsOneWidget);
    expect(findRichText('Owner update (edited)'), findsNothing);

    // The relay boundary supplies a newer edit while ownership is pending.
    session.emit(edit(other.public, 'Other update', 3000));
    await tester.pumpAndSettle();
    expect(findRichText('Original agent message'), findsOneWidget);
    expect(findRichText('Other update (edited)'), findsNothing);

    profiles.complete([
      _profileEvent(
        id: 'agent-owner-profile',
        pubkey: agent.public,
        createdAt: 1,
        name: 'Helper',
        tags: [_authTag(owner, agent.public)],
      ),
    ]);
    await tester.pumpAndSettle();
    expect(container.read(agentOwnersProvider).requireValue, {
      agent.public: owner.public,
    });
    expect(findRichText('Owner update (edited)'), findsOneWidget);
    expect(findRichText('Other update (edited)'), findsNothing);

    // Refresh through the real owner provider; removing its verified entry
    // restores the original message without waiting for another message.
    session.profiles = Future.value(const []);
    container.invalidate(agentOwnersProvider);
    await tester.pumpAndSettle();
    expect(container.read(agentOwnersProvider).requireValue, isEmpty);
    expect(findRichText('Original agent message'), findsOneWidget);
    expect(findRichText('Author update (edited)'), findsOneWidget);
    expect(findRichText('Owner update (edited)'), findsNothing);
    expect(findRichText('Other update (edited)'), findsNothing);
  });
}

class _EditRelaySession extends RelaySessionNotifier {
  _EditRelaySession(this.agent, this.profiles, this.events);

  final String agent;
  Future<List<NostrEvent>> profiles;
  final List<NostrEvent> events;
  final _listeners = <(NostrFilter, void Function(NostrEvent))>[];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filter.kinds.contains(10100)) {
      return [
        NostrEvent(
          id: 'agent-directory',
          pubkey: agent,
          createdAt: 1,
          kind: 10100,
          tags: const [],
          content: '{"display_name":"Helper"}',
          sig: '',
        ),
      ];
    }
    if (filter.kinds.contains(0)) return profiles;
    return events.where((event) => filter.kinds.contains(event.kind)).toList();
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async => const [];

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    final listener = (filter, onEvent);
    _listeners.add(listener);
    return () => _listeners.remove(listener);
  }

  void emit(NostrEvent event) {
    for (final (filter, listener) in [..._listeners]) {
      if (filter.kinds.contains(event.kind)) listener(event);
    }
  }
}
