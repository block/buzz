part of 'channels_provider_test.dart';

void _unreadReadinessCases() {
  test(
    'retired unread completion cannot overwrite destination settlement',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, 'me')],
        metadata: [_meta(id: _channelA, name: 'Alpha')],
      )..pauseNextUnreadCatchUpQuery();
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await session.nextUnreadCatchUpQueryStarted;
      final notifier = container.read(channelsProvider.notifier);

      session.memberships = [_membership(_channelB, 'me')];
      session.metadata = [_meta(id: _channelB, name: 'Bravo')];
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://bravo.example');
      await container.read(channelsProvider.future);
      await notifier.waitForUnreadCatchUp();

      // Alpha finishes after Bravo has already settled. Its finally path must
      // not replace Bravo's generation, even when Alpha's request failed.
      session.failClaimedUnreadCatchUpQuery = true;
      session.resumePausedUnreadCatchUpQuery();
      await _settle();
      var settled = false;
      unawaited(notifier.waitForUnreadCatchUp().then((_) => settled = true));
      await _settle();
      expect(
        settled,
        isTrue,
        reason: 'Retired history overwrote destination settlement',
      );
      expect(
        container.read(channelsProvider).requireValue.single.id,
        _channelB,
      );
    },
  );

  for (final failure in ['deadline', 'exception', 'read-state unavailable']) {
    test('destination unread $failure permits degraded landing', () async {
      final session = _UnreadFailureSession(failure);
      final container = ProviderContainer(
        retry: (_, _) => null,
        overrides: [
          appLifecycleProvider.overrideWith(() => _FakeAppLifecycleNotifier()),
          relaySessionProvider.overrideWith(() => session),
          myPubkeyProvider.overrideWith(
            (ref) => ref.watch(_testPubkeyProvider),
          ),
          if (failure == 'read-state unavailable')
            readStateProvider.overrideWith(
              _UnavailableDestinationReadState.new,
            ),
        ],
      );
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      final notifier = container.read(channelsProvider.notifier);
      await notifier.waitForUnreadCatchUp();

      session.memberships = [_membership(_channelB, 'me')];
      session.metadata = [_meta(id: _channelB, name: 'Bravo')];
      session.failDestination = true;
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://bravo.example');
      expect(
        (await container.read(channelsProvider.future)).single.id,
        _channelB,
      );
      var landed = false;
      unawaited(notifier.waitForUnreadCatchUp().then((_) => landed = true));

      if (failure != 'read-state unavailable') {
        await session.started.future;
        await _settle();
        expect(landed, isFalse);
        session.emit(
          const NostrEvent(
            id: 'retained-mention',
            pubkey: 'alice',
            createdAt: 50,
            kind: 9,
            tags: [
              ['h', _channelB],
              ['p', 'me'],
            ],
            content: 'Hi',
            sig: 'sig',
          ),
        );
        expect(
          notifier.observedUnreadEventsByChannel[_channelB],
          contains('retained-mention'),
        );
        session.release.complete();
      }
      // No 20-second presentation timeout or unrelated reconciliation needed.
      await _settle();
      expect(
        landed,
        isTrue,
        reason: 'Terminal history must settle this destination',
      );
      expect(
        container.read(channelsProvider).requireValue.single.id,
        _channelB,
      );
      if (failure != 'read-state unavailable') {
        expect(
          notifier.observedUnreadEventsByChannel[_channelB],
          contains('retained-mention'),
        );
      }
    });
  }
}

class _UnreadFailureSession extends _FakeRelaySession {
  _UnreadFailureSession(this.failure)
    : super(
        memberships: [_membership(_channelA, 'me')],
        metadata: [_meta(id: _channelA, name: 'Alpha')],
      );

  final String failure;
  bool failDestination = false;
  final started = Completer<void>();
  final release = Completer<void>();

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (failDestination &&
        filters.isNotEmpty &&
        filters.every((filter) => filter.since != null)) {
      started.complete();
      await release.future;
      if (failure == 'deadline') {
        throw RelayException(503, '{"error":"query timed out"}');
      }
      throw StateError('Unread history unavailable');
    }
    return super.queryRelay(filters, timeout: timeout);
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (failDestination &&
        filter.since != null &&
        filter.kinds.contains(EventKind.streamMessageV2)) {
      throw StateError('Unread fallback unavailable');
    }
    return super.fetchHistory(filter, timeout: timeout);
  }
}

class _UnavailableDestinationReadState extends ReadStateNotifier {
  @override
  ReadStateState build() {
    if (ref.watch(relayConfigProvider).baseUrl == 'https://bravo.example') {
      throw StateError('Destination read state unavailable');
    }
    return const ReadStateState.inert();
  }
}
