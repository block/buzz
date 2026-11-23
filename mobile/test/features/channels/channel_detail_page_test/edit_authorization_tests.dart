part of '../channel_detail_page_test.dart';

const _editRelay = 'https://relay.example';
final _trustedRelayKey = 'e' * 64;
final _rotatedRelayKey = 'f' * 64;

void _editAuthorizationTests() {
  testWidgets(
    'owner edits and the Edit menu follow the signed profile, not the agent '
    'directory',
    (tester) async {
      final fixture = _EditFixture();
      // Answers to NIP-11 requests, in order.
      final relayAnswers = <Completer<String?>>[];
      final relayRequests = <String>[];
      final session = _EditRelaySession(
        profiles: [fixture.agentProfile],
        events: [
          fixture.agentMessage,
          fixture.ownerEdit,
          fixture.agentDiff,
          fixture.ownerDiffEdit,
          fixture.humanMessage,
        ],
      );
      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          useRealEditProviders: true,
          userCacheNotifier: UserCacheNotifier(),
          relaySessionNotifier: session,
          relayConfigNotifier: _EditRelayConfig(fixture.owner.secret),
          signingPubkey: fixture.owner.public,
          loadRelayAuthor: (url) {
            relayRequests.add(url);
            return (relayAnswers..add(Completer())).last.future;
          },
          extraOverrides: [
            savedRelayAuthorProvider.overrideWith((ref) async => null),
          ],
        ),
      );
      await tester.pumpAndSettle();

      // The agent is in no directory: its signed profile alone proves the
      // owner, for chat messages and stream diffs alike.
      expect(session.directoryReads, greaterThan(0));
      expect(findRichText('Owner update (edited)'), findsOneWidget);
      expect(findRichText('Diff update (edited)'), findsOneWidget);
      // No edit needs the relay's key, so it is not requested.
      expect(relayRequests, isEmpty);

      // The Edit menu offers what renders: the owner may edit the agent's
      // message, and not a stranger's.
      await _expectEditMenu(tester, 'Owner update (edited)', isTrue);
      await _expectEditMenu(tester, 'Human message', isFalse);

      // A relay-attributed edit asks for the relay's key once, and applies
      // after the key matches the target's signer.
      session.emit(fixture.relayMessage(_trustedRelayKey));
      session.emit(fixture.humanRelayEdit);
      await tester.pumpAndSettle();
      expect(relayRequests, [_editRelay]);
      expect(findRichText('Relay message'), findsOneWidget);
      relayAnswers.last.complete(_trustedRelayKey);
      await tester.pumpAndSettle();
      expect(findRichText('Relay update (edited)'), findsOneWidget);

      // A reconnect keeps every edit while the key is read again.
      session.setStatus(SessionStatus.reconnecting);
      await tester.pumpAndSettle();
      expect(relayRequests, hasLength(1));
      session.setStatus(SessionStatus.connected);
      await tester.pumpAndSettle();
      expect(relayRequests, hasLength(2));
      for (final text in [
        'Owner update (edited)',
        'Diff update (edited)',
        'Relay update (edited)',
      ]) {
        expect(findRichText(text), findsOneWidget, reason: text);
      }

      // A rotated key supersedes the old one as soon as it is read.
      relayAnswers.last.complete(_rotatedRelayKey);
      await tester.pumpAndSettle();
      expect(findRichText('Relay message'), findsOneWidget);
      expect(findRichText('Relay update (edited)'), findsNothing);
      expect(findRichText('Owner update (edited)'), findsOneWidget);

      // A newer profile without the attestation removes the owner's edits
      // and the Edit menu together.
      final container = ProviderScope.containerOf(
        tester.element(find.byType(ChannelDetailPage)),
      );
      container
          .read(userCacheProvider.notifier)
          .cacheProfileEvent(fixture.agentProfileWithoutOwner);
      await tester.pumpAndSettle();
      expect(findRichText('Original agent message'), findsOneWidget);
      expect(findRichText('Original diff'), findsOneWidget);
      expect(findRichText('Owner update (edited)'), findsNothing);
      await _expectEditMenu(tester, 'Original agent message', isFalse);
    },
  );

  testWidgets('an offline reopen applies edits from stored evidence', (
    tester,
  ) async {
    final fixture = _EditFixture();
    // The last session saved the agent's signed profile and the relay key.
    final storage = ProfileSnapshotStorage(_testPrefs);
    await storage.write(storage.capture(fixture.owner.public, _editRelay), [
      parseProfileEvent(fixture.agentProfile),
    ]);
    addTearDown(() => storage.remove(fixture.owner.public, _editRelay));
    final relayRequests = <String>[];
    final session = _EditRelaySession(
      profiles: const [],
      events: const [],
      status: SessionStatus.disconnected,
    );
    await tester.pumpWidget(
      _buildTestable(
        messages: [
          fixture.agentMessage,
          fixture.ownerEdit,
          fixture.relayMessage(_trustedRelayKey),
          fixture.humanRelayEdit,
        ],
        userCacheNotifier: UserCacheNotifier(),
        relaySessionNotifier: session,
        relayConfigNotifier: _EditRelayConfig(fixture.owner.secret),
        signingPubkey: fixture.owner.public,
        loadRelayAuthor: (url) async {
          relayRequests.add(url);
          return _trustedRelayKey;
        },
        extraOverrides: [
          savedRelayAuthorProvider.overrideWith(
            (ref) async => _trustedRelayKey,
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(findRichText('Owner update (edited)'), findsOneWidget);
    expect(findRichText('Relay update (edited)'), findsOneWidget);
    expect(relayRequests, isEmpty);
    await _expectEditMenu(tester, 'Owner update (edited)', isTrue);
  });
}

/// Long-presses [text] and checks whether the actions offer Edit.
Future<void> _expectEditMenu(
  WidgetTester tester,
  String text,
  Matcher offered,
) async {
  await tester.longPress(findRichText(text));
  await tester.pumpAndSettle();
  expect(
    find.text('Edit message').evaluate().isNotEmpty,
    offered,
    reason: text,
  );
  await tester.tapAt(const Offset(8, 8));
  await tester.pumpAndSettle();
  expect(find.text('Edit message'), findsNothing);
}

/// The viewer owns an agent outside the agent directory; a human posts too.
class _EditFixture {
  final owner = nostr.Keys.generate();
  final agent = nostr.Keys.generate();
  final human = nostr.Keys.generate();

  late final agentProfile = _signedKind0(tags: [_authTag(owner, agent.public)]);
  late final agentProfileWithoutOwner = _signedKind0(createdAt: 2);

  NostrEvent _signedKind0({
    int createdAt = 1,
    List<List<String>> tags = const [],
  }) => NostrEvent.fromJson(
    signNostrEvent(
      kind: 0,
      content: jsonEncode({'name': 'Helper'}),
      secretKey: agent.secret,
      createdAt: createdAt,
      tags: tags,
    ).toMap(),
  );

  late final agentMessage = _textMsg(
    id: threadIdA,
    pubkey: agent.public,
    content: 'Original agent message',
  );
  late final agentDiff = NostrEvent(
    id: threadIdMid,
    pubkey: agent.public,
    createdAt: 1001,
    kind: EventKind.streamMessageDiff,
    tags: const [
      ['h', _channelId],
    ],
    content: 'Original diff',
    sig: '',
  );
  late final humanMessage = _textMsg(
    id: threadIdOwner,
    pubkey: human.public,
    content: 'Human message',
    createdAt: 1002,
  );
  late final ownerEdit = _edit(owner.public, threadIdA, 'Owner update', 2000);
  late final ownerDiffEdit = _edit(
    owner.public,
    threadIdMid,
    'Diff update',
    2001,
  );
  late final humanRelayEdit = _edit(
    human.public,
    threadIdRoot,
    'Relay update',
    2002,
  );

  /// A message [relayKey] signed for the human.
  NostrEvent relayMessage(String relayKey) => _textMsg(
    id: threadIdRoot,
    pubkey: relayKey,
    content: 'Relay message',
    createdAt: 1003,
    extraTags: [
      ['actor', human.public],
    ],
  );

  NostrEvent _edit(String signer, String target, String content, int time) =>
      NostrEvent(
        id: 'edit-$time',
        pubkey: signer,
        createdAt: time,
        kind: EventKind.streamMessageEdit,
        tags: [
          ['h', _channelId],
          ['e', target],
        ],
        content: content,
        sig: '',
      );
}

class _EditRelayConfig extends RelayConfigNotifier {
  _EditRelayConfig(this.secret);

  final String secret;

  @override
  RelayConfig build() => RelayConfig(
    baseUrl: _editRelay,
    nsec: nostr.Nip19.encode(prefix: nostr.Nip19Prefix.nsec, data: secret),
  );
}

class _EditRelaySession extends RelaySessionNotifier {
  _EditRelaySession({
    required this.profiles,
    required this.events,
    this.status = SessionStatus.connected,
  });

  final List<NostrEvent> profiles;
  final List<NostrEvent> events;
  final SessionStatus status;
  var directoryReads = 0;
  final _listeners = <(NostrFilter, void Function(NostrEvent))>[];

  @override
  SessionState build() => SessionState(status: status);

  void setStatus(SessionStatus status) => state = SessionState(status: status);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (state.status != SessionStatus.connected) {
      throw StateError('offline');
    }
    if (filter.kinds.contains(10100)) {
      directoryReads++;
      return const [];
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
