part of 'channels_provider_test.dart';

void _unreadDeletionCases() {
  for (final source in ['live deletion', 'catch-up deletion', 'live edit']) {
    test('closed saved channel reconciles a $source', () async {
      const scope = (communityId: 'community', pubkey: 'me');
      const page = (channelId: _channelA, rootId: '');
      final directory = await Directory.systemTemp.createTemp(
        'buzz-closed-chat',
      );
      addTearDown(() => directory.delete(recursive: true));
      var rootReads = 0;
      ProviderContainer open(_FakeRelaySession session) => _buildContainer(
        session: session,
        overrides: [
          savedMessageScopeProvider.overrideWithValue(scope),
          savedMessageStoreProvider.overrideWith(
            (ref) => SavedMessageStore(
              root: () async {
                rootReads++;
                return directory;
              },
              diagnostics: const NoopDiagnosticReporter(),
            ),
          ),
        ],
      );
      NostrEvent signed(
        int kind,
        String content, {
        String? target,
      }) => NostrEvent.fromJson(
        signNostrEvent(
          secretKey:
              '0000000000000000000000000000000000000000000000000000000000000001',
          kind: kind,
          content: content,
          tags: [
            ['h', _channelA],
            if (target != null) ['e', target],
          ],
        ).toMap(),
      );
      final original = signed(EventKind.streamMessage, 'Original text');
      final neighbor = signed(EventKind.streamMessage, 'Neighbor');
      final edit = source == 'live edit';
      final mutation = signed(
        edit ? EventKind.streamMessageEdit : EventKind.deletion,
        edit ? 'Updated text' : '',
        target: original.id,
      );
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, 'me')],
        metadata: [_meta(id: _channelA, name: 'general')],
      );
      final online = open(session);
      addTearDown(online.dispose);
      final store = online.read(savedMessageStoreProvider);
      await store.save(store.capture(scope, page), [
        original,
        neighbor,
      ], relayAuthor: null);
      await online.read(channelsProvider.future);
      final notifier = online.read(channelsProvider.notifier);
      await notifier.waitForUnreadCatchUp();
      rootReads = 0;

      if (source == 'catch-up deletion') {
        session.recentMessages = [mutation];
        await notifier.refresh();
        await notifier.waitForUnreadCatchUp();
      } else {
        session.emit(mutation);
        if (!edit) {
          for (var i = 0; i < 32; i++) {
            session.emit(signed(EventKind.deletion, '', target: 'missing-$i'));
          }
        }
      }
      await store.touch(store.capture(scope, page));
      final queuedOperations = rootReads;

      final offlineSession = _FakeRelaySession(memberships: [], metadata: []);
      final offline = open(offlineSession);
      addTearDown(offline.dispose);
      offline.read(relaySessionProvider);
      offlineSession.setStatus(SessionStatus.disconnected);
      final preview = savedChannelPreviewProvider(_channelA);
      offline.listen(preview, (_, _) {});
      expect((await offline.read(preview.future))?.events, [neighbor]);
      expect(
        queuedOperations,
        2,
        reason: 'one removal batch and one queue barrier',
      );
    });
  }

  for (final format in [(5, true), (5, false), (9005, true)]) {
    test(
      'deleted mentions stop unread (kind ${format.$1}, h ${format.$2})',
      () async {
        // Timestamps below are offsets from a recent base: a repeated
        // catch-up starts within the relay's drift window, and the relay
        // stores nothing older than that since then.
        final base = DateTime.now().millisecondsSinceEpoch ~/ 1000 - 100;
        NostrEvent message(String id, int at) => NostrEvent(
          id: id,
          pubkey: 'alice',
          createdAt: base + at,
          kind: 9,
          tags: const [
            ['h', _channelA],
            ['p', 'me'],
          ],
          content: 'hello',
          sig: 'sig',
        );
        NostrEvent deletion(String id, String target, int at) => NostrEvent(
          id: id,
          pubkey: 'alice',
          createdAt: base + at,
          kind: format.$1,
          tags: [
            if (format.$2) ['h', _channelA],
            ['e', target],
          ],
          content: '',
          sig: 'sig',
        );
        final session = _FakeRelaySession(
          memberships: [
            _membership(_channelA, 'me'),
            _membership(_channelB, 'me'),
          ],
          metadata: [
            _meta(id: _channelA, name: 'general'),
            _meta(id: _channelB, name: 'other'),
          ],
          enforceHistoryLimits: true,
          deletionChannels: {
            'missed-deletion': _channelA,
            for (var i = 0; i < 600; i++) 'unreact-a-$i': _channelA,
            for (var i = 0; i < 1000; i++) 'unreact-b-$i': _channelB,
          },
        );
        final container = _buildContainer(session: session);
        addTearDown(container.dispose);
        await container.read(channelsProvider.future);
        await _settle();
        final mention = message('deleted-mention', 50);
        session.emit(mention);
        await _settle();
        final notifier = container.read(channelsProvider.notifier);
        expect(
          notifier.observedUnreadEventsByChannel[_channelA],
          contains(mention.id),
        );
        session.emit(deletion('delete', mention.id, 60));
        await _settle();
        expect(
          notifier.observedUnreadEventsByChannel[_channelA] ?? {},
          isEmpty,
        );
        expect(notifier.latestObservedByChannel, isNot(contains(_channelA)));

        // A late history result must not restore the deleted message.
        session.recentMessages = [mention];
        await notifier.refresh();
        await notifier.waitForUnreadCatchUp();
        expect(
          notifier.observedUnreadEventsByChannel[_channelA] ?? {},
          isEmpty,
        );

        // Deletion can also arrive before its message.
        session.emit(deletion('delete-before-message', 'late-mention', 70));
        session.emit(message('late-mention', 65));
        await _settle();
        expect(
          notifier.observedUnreadEventsByChannel[_channelA] ?? {},
          isEmpty,
        );

        // Recover a deletion missed offline. The batch order must not matter.
        final missed = message('offline-mention', 80);
        session.emit(missed);
        await _settle();
        expect(
          notifier.observedUnreadEventsByChannel[_channelA],
          contains(missed.id),
        );
        session.recentMessages = [
          missed,
          deletion('missed-deletion', missed.id, 90),
        ];
        await notifier.refresh();
        await notifier.waitForUnreadCatchUp();
        expect(
          notifier.observedUnreadEventsByChannel[_channelA] ?? {},
          isEmpty,
        );
        expect(notifier.latestObservedByChannel, isNot(contains(_channelA)));

        // Reaction removals must neither consume the message window nor evict
        // deletion evidence needed by this same multi-channel catch-up batch.
        NostrEvent unreact(String id, int at) => NostrEvent(
          id: id,
          pubkey: 'alice',
          createdAt: base + at,
          kind: EventKind.deletion,
          tags: [
            ['e', 'reaction-$id'],
          ],
          content: '',
          sig: 'sig',
        );
        session.recentMessages = [
          missed,
          deletion('missed-deletion', missed.id, 90),
          for (var i = 0; i < 1000; i++) unreact('unreact-b-$i', 2000 + i),
        ];
        await notifier.refresh();
        await notifier.waitForUnreadCatchUp();
        expect(
          notifier.observedUnreadEventsByChannel[_channelA] ?? {},
          isEmpty,
          reason: 'this batch must retain all of its own deletion evidence',
        );

        session.recentMessages = [
          missed,
          deletion('missed-deletion', missed.id, 90),
          for (var i = 0; i < 600; i++) message('kept-$i', 100 + i),
          for (var i = 0; i < 600; i++) unreact('unreact-a-$i', 1000 + i),
          for (var i = 0; i < 1000; i++) unreact('unreact-b-$i', 2000 + i),
        ];
        await notifier.refresh();
        await notifier.waitForUnreadCatchUp();
        expect(
          notifier.observedUnreadEventsByChannel[_channelA]?.keys,
          unorderedEquals([for (var i = 0; i < 600; i++) 'kept-$i']),
          reason:
              'all surviving mentions remain unread, but deleted ones do not',
        );
      },
    );
  }
}
