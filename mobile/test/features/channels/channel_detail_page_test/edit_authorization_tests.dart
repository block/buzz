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
    // Its long about text fits once, but not beside the signed copy.
    final storage = ProfileSnapshotStorage(_testPrefs);
    await storage.write(storage.capture(fixture.owner.public, _editRelay), [
      parseProfileEvent(fixture.longAgentProfile),
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

  testWidgets(
    'a restored owner without its signed profile cannot edit, in the channel '
    'or a thread',
    (tester) async {
      final fixture = _EditFixture();
      // The restored agent's entry kept its owner but lost its signed copy.
      final storage = ProfileSnapshotStorage(_testPrefs);
      await storage.write(storage.capture(fixture.owner.public, _editRelay), [
        parseProfileEvent(fixture.agentProfile),
        fixture.restoredWithoutEvidence,
      ]);
      addTearDown(() => storage.remove(fixture.owner.public, _editRelay));
      final replies = [
        _textMsg(
          id: threadIdR1,
          pubkey: fixture.agent.public,
          content: 'Signed agent reply',
          createdAt: 1500,
          extraTags: [
            ['e', threadIdA, '', 'reply'],
          ],
        ),
        _textMsg(
          id: threadIdRetained,
          pubkey: fixture.restored.public,
          content: 'Restored agent reply',
          createdAt: 1501,
          extraTags: [
            ['e', threadIdA, '', 'reply'],
          ],
        ),
      ];
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            fixture.agentMessage,
            fixture.ownerEdit,
            fixture.restoredMessage,
            fixture.restoredOwnerEdit,
          ],
          threadReplies: {threadIdA: replies},
          userCacheNotifier: UserCacheNotifier(),
          relaySessionNotifier: _EditRelaySession(
            profiles: const [],
            events: const [],
            status: SessionStatus.disconnected,
          ),
          relayConfigNotifier: _EditRelayConfig(fixture.owner.secret),
          signingPubkey: fixture.owner.public,
        ),
      );
      await tester.pumpAndSettle();

      expect(findRichText('Owner update (edited)'), findsOneWidget);
      expect(findRichText('Restored agent message'), findsOneWidget);
      await _expectEditMenu(tester, 'Owner update (edited)', isTrue);
      await _expectEditMenu(tester, 'Restored agent message', isFalse);

      final head = formatTimeline(
        [fixture.agentMessage, fixture.ownerEdit],
        agentOwners: {fixture.agent.public: fixture.owner.public},
      ).single;
      unawaited(
        Navigator.of(tester.element(find.byType(ChannelDetailPage))).push(
          MaterialPageRoute<void>(
            builder: (_) => ThreadDetailPage(
              threadHead: head,
              allMessages: [head],
              channelId: _channelId,
              currentPubkey: fixture.owner.public,
              isMember: true,
              isArchived: false,
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byType(ThreadDetailPage), findsOneWidget);
      await _expectEditMenu(tester, 'Signed agent reply', isTrue);
      await _expectEditMenu(tester, 'Restored agent reply', isFalse);
    },
  );

  testWidgets('a profile signed by another key does not make an owner', (
    tester,
  ) async {
    final fixture = _EditFixture();
    await tester.pumpWidget(
      _buildTestable(
        messages: [fixture.restoredMessage, fixture.restoredOwnerEdit],
        users: {
          // A valid attestation for another key, filed under this agent.
          fixture.restored.public: UserProfile(
            pubkey: fixture.restored.public,
            ownerPubkey: fixture.owner.public,
            ownerEvidence: fixture.agentProfile,
          ),
        },
        signingPubkey: fixture.owner.public,
      ),
    );
    await tester.pumpAndSettle();

    expect(findRichText('Restored agent message'), findsOneWidget);
    expect(findRichText('Restored update (edited)'), findsNothing);
    await _expectEditMenu(tester, 'Restored agent message', isFalse);
  });

  testWidgets(
    'the saved relay key applies until a key is read, then the read one does',
    (tester) async {
      final fixture = _EditFixture();
      final live = Completer<String?>();
      await tester.pumpWidget(
        _buildTestable(
          messages: [
            fixture.relayMessage(_trustedRelayKey),
            fixture.humanRelayEdit,
          ],
          relaySessionNotifier: _EditRelaySession(
            profiles: const [],
            events: const [],
          ),
          relayConfigNotifier: _EditRelayConfig(fixture.owner.secret),
          signingPubkey: fixture.owner.public,
          loadRelayAuthor: (_) => live.future,
          extraOverrides: [
            savedRelayAuthorProvider.overrideWith(
              (ref) async => _trustedRelayKey,
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();
      expect(findRichText('Relay update (edited)'), findsOneWidget);

      live.complete(_rotatedRelayKey);
      await tester.pumpAndSettle();
      expect(findRichText('Relay message'), findsOneWidget);
      expect(findRichText('Relay update (edited)'), findsNothing);
    },
  );

  testWidgets('Edit permission is read from the profile when actions open', (
    tester,
  ) async {
    final fixture = _EditFixture();
    final cache = _FakeUserCacheNotifier({
      fixture.agent.public: parseProfileEvent(fixture.agentProfile).profile,
    });
    final reply = _textMsg(
      id: threadIdR1,
      pubkey: fixture.agent.public,
      content: 'Agent reply',
      createdAt: 1500,
      extraTags: [
        ['e', threadIdA, '', 'reply'],
      ],
    );
    verifiedOwnerChecks = 0;
    await tester.pumpWidget(
      _buildTestable(
        messages: [fixture.agentMessage],
        threadReplies: {
          threadIdA: [reply],
        },
        userCacheNotifier: cache,
        signingPubkey: fixture.owner.public,
      ),
    );
    await tester.pumpAndSettle();

    // Without edits, showing the channel checks no signed profile.
    expect(findRichText('Original agent message'), findsOneWidget);
    expect(verifiedOwnerChecks, 0);
    await _expectEditMenu(tester, 'Original agent message', isTrue);
    expect(verifiedOwnerChecks, 1);

    // A newer profile without the attestation is what the next menu reads,
    // in the channel and in a thread.
    cache.replace(parseProfileEvent(fixture.agentProfileWithoutOwner).profile);
    await tester.pump();
    await _expectEditMenu(tester, 'Original agent message', isFalse);
    final head = formatTimeline([fixture.agentMessage]).single;
    unawaited(
      Navigator.of(tester.element(find.byType(ChannelDetailPage))).push(
        MaterialPageRoute<void>(
          builder: (_) => ThreadDetailPage(
            threadHead: head,
            allMessages: [head],
            channelId: _channelId,
            currentPubkey: fixture.owner.public,
            isMember: true,
            isArchived: false,
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    await _expectEditMenu(tester, 'Agent reply', isFalse);
  });

  testWidgets('a restored owner hint without a signed profile is asked for '
      'again when actions open', (tester) async {
    final fixture = _EditFixture();
    final cache = _RecordingUserCache({
      fixture.restored.public: fixture.restoredWithoutEvidence.profile,
    });
    await tester.pumpWidget(
      _buildTestable(
        messages: [fixture.restoredMessage],
        userCacheNotifier: cache,
        signingPubkey: fixture.owner.public,
      ),
    );
    await tester.pumpAndSettle();
    expect(cache.requested, isEmpty);

    await _expectEditMenu(tester, 'Restored agent message', isFalse);
    expect(cache.requested, [fixture.restored.public]);
  });

  testWidgets('image actions read Edit permission when they open', (
    tester,
  ) async {
    final fixture = _EditFixture();
    final cache = _FakeUserCacheNotifier({
      fixture.agent.public: parseProfileEvent(fixture.agentProfile).profile,
    });
    tester.view.physicalSize = const Size(400, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    const first = 'https://example.org/media/first.png';
    const second = 'https://example.org/media/second.png';
    verifiedOwnerChecks = 0;
    await tester.pumpWidget(
      _buildTestable(
        messages: [
          _textMsg(
            id: threadIdA,
            pubkey: fixture.agent.public,
            content: '![First]($first)\n![Second]($second)',
            extraTags: const [
              ['imeta', 'url $first', 'm image/png'],
              ['imeta', 'url $second', 'm image/png'],
            ],
          ),
        ],
        userCacheNotifier: cache,
        signingPubkey: fixture.owner.public,
      ),
    );
    await tester.pumpAndSettle();
    expect(verifiedOwnerChecks, 0);

    await tester.tap(find.byKey(const ValueKey('message-media-stack')));
    await tester.pumpAndSettle();
    final more = find.byKey(
      const ValueKey('message-media-image-viewer-more-actions'),
    );
    await tester.tap(more);
    await tester.pumpAndSettle();
    expect(find.text('Delete message'), findsOneWidget);
    expect(verifiedOwnerChecks, 1);

    // The next opening reads the newer profile without the attestation.
    Navigator.of(tester.element(find.text('Delete message'))).pop();
    await tester.pumpAndSettle();
    cache.replace(parseProfileEvent(fixture.agentProfileWithoutOwner).profile);
    await tester.pump();
    await tester.tap(more);
    await tester.pumpAndSettle();
    expect(find.text('Save image'), findsOneWidget);
    expect(find.text('Delete message'), findsNothing);
  });

  testWidgets('owner profiles are asked for again when the relay reconnects', (
    tester,
  ) async {
    final fixture = _EditFixture();
    // The agent has no stored profile; the restored one has no signed copy.
    final storage = ProfileSnapshotStorage(_testPrefs);
    await storage.write(storage.capture(fixture.owner.public, _editRelay), [
      fixture.restoredWithoutEvidence,
    ]);
    addTearDown(() => storage.remove(fixture.owner.public, _editRelay));
    final session = _EditRelaySession(
      profiles: [fixture.agentProfile, fixture.restoredProfile],
      events: const [],
      failedProfileReads: 1,
    );
    await tester.pumpWidget(
      _buildTestable(
        messages: [
          fixture.agentMessage,
          fixture.ownerEdit,
          fixture.restoredMessage,
          fixture.restoredOwnerEdit,
        ],
        userCacheNotifier: UserCacheNotifier(),
        relaySessionNotifier: session,
        relayConfigNotifier: _EditRelayConfig(fixture.owner.secret),
        signingPubkey: fixture.owner.public,
      ),
    );
    await tester.pumpAndSettle();

    // The first request for both timed out. The agent's row asks again for
    // its missing profile, but nothing else asks for the restored one.
    expect(session.profileReads, greaterThan(1));
    expect(findRichText('Restored agent message'), findsOneWidget);

    session.setStatus(SessionStatus.reconnecting);
    await tester.pumpAndSettle();
    session.setStatus(SessionStatus.connected);
    await tester.pumpAndSettle();
    expect(findRichText('Owner update (edited)'), findsOneWidget);
    expect(findRichText('Restored update (edited)'), findsOneWidget);
    final reads = session.profileReads;

    // Both are answered now, so a later reconnect does not ask again.
    session.setStatus(SessionStatus.reconnecting);
    await tester.pumpAndSettle();
    session.setStatus(SessionStatus.connected);
    await tester.pumpAndSettle();
    expect(session.profileReads, reads);
  });

  // A head outside the relay's thread history is formatted from the saved
  // page, with the edit authority its own edits need.
  for (final attributed in [false, true]) {
    testWidgets('a saved thread head shows its '
        '${attributed ? 'relay attributed author' : 'owner'} edit', (
      tester,
    ) async {
      final fixture = _EditFixture();
      final head = attributed
          ? fixture.relayMessage(_trustedRelayKey)
          : fixture.agentMessage;
      final relayKey = Completer<String?>();
      await tester.pumpWidget(
        _buildTestable(
          messages: const [],
          savedThreadPreview: Future.value([
            head,
            attributed ? fixture.humanRelayEdit : fixture.ownerEdit,
          ]),
          initialThreadRootId: head.id,
          threadReplies: {
            head.id: [
              _textMsg(
                id: threadIdR1,
                pubkey: fixture.human.public,
                content: 'Relay reply',
                createdAt: 1500,
                extraTags: [
                  ['e', head.id, '', 'reply'],
                ],
              ),
            ],
          },
          users: {
            fixture.agent.public: parseProfileEvent(
              fixture.agentProfile,
            ).profile,
          },
          relaySessionNotifier: _EditRelaySession(
            profiles: const [],
            events: const [],
          ),
          relayConfigNotifier: _EditRelayConfig(fixture.owner.secret),
          signingPubkey: fixture.owner.public,
          loadRelayAuthor: (_) => relayKey.future,
          extraOverrides: [
            savedRelayAuthorProvider.overrideWith((ref) async => null),
          ],
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byType(ThreadDetailPage), findsOneWidget);
      expect(findRichText('Relay reply'), findsOneWidget);

      if (attributed) {
        expect(findRichText('Relay message'), findsOneWidget);
        relayKey.complete(_trustedRelayKey);
        await tester.pumpAndSettle();
        expect(findRichText('Relay update (edited)'), findsOneWidget);
      } else {
        expect(findRichText('Owner update (edited)'), findsOneWidget);
        await _expectEditMenu(tester, 'Owner update (edited)', isTrue);
      }
    });
  }
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

/// Records the profiles asked for with [get].
class _RecordingUserCache extends _FakeUserCacheNotifier {
  _RecordingUserCache(super.users);

  final requested = <String>[];

  @override
  UserProfile? get(String pubkey) {
    requested.add(pubkey.toLowerCase());
    return super.get(pubkey);
  }
}

/// The viewer owns an agent outside the agent directory; a human posts too.
class _EditFixture {
  final owner = nostr.Keys.generate();
  final agent = nostr.Keys.generate();
  final human = nostr.Keys.generate();

  /// An agent whose stored profile names [owner] without its signed copy.
  final restored = nostr.Keys.generate();
  late final restoredProfile = _signedKind0(
    signer: restored,
    tags: [_authTag(owner, restored.public)],
  );
  late final restoredWithoutEvidence = (
    profile: UserProfile(
      pubkey: restored.public,
      displayName: 'Restored',
      ownerPubkey: owner.public,
    ),
    createdAt: restoredProfile.createdAt,
    eventId: restoredProfile.id,
  );
  late final restoredMessage = _textMsg(
    id: threadIdUnrelated,
    pubkey: restored.public,
    content: 'Restored agent message',
    createdAt: 1004,
  );
  late final restoredOwnerEdit = _edit(
    owner.public,
    threadIdUnrelated,
    'Restored update',
    2003,
  );

  late final agentProfile = _signedKind0(tags: [_authTag(owner, agent.public)]);
  late final longAgentProfile = _signedKind0(
    tags: [_authTag(owner, agent.public)],
    about: 'x' * 9000,
  );
  late final agentProfileWithoutOwner = _signedKind0(createdAt: 2);

  NostrEvent _signedKind0({
    nostr.Keys? signer,
    int createdAt = 1,
    List<List<String>> tags = const [],
    String? about,
  }) => NostrEvent.fromJson(
    signNostrEvent(
      kind: 0,
      content: jsonEncode({'name': 'Helper', 'about': ?about}),
      secretKey: (signer ?? agent).secret,
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
    this.failedProfileReads = 0,
  });

  final List<NostrEvent> profiles;
  final List<NostrEvent> events;
  final SessionStatus status;

  /// Profile reads that time out before any succeeds.
  final int failedProfileReads;
  var directoryReads = 0;
  var profileReads = 0;
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
    if (filter.kinds.contains(0)) {
      if (++profileReads <= failedProfileReads) {
        throw TimeoutException('profiles');
      }
      return [
        for (final profile in profiles)
          if (filter.authors?.contains(profile.pubkey) ?? true) profile,
      ];
    }
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
