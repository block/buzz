part of 'channels_provider_test.dart';

void _testWithClock(String name, void Function(FakeAsync) body) {
  test(name, () => fakeAsync(body));
}

void _terminalSubscriptionTests() {
  for (final beforeReady in [true, false]) {
    _testWithClock(
      'persistent terminal rejection ${beforeReady ? 'before' : 'after'} '
      'readiness cannot amplify refreshes or retries',
      (clock) {
        final session = _TerminalRelaySession(beforeReady: beforeReady);
        final logs = <String>[];
        final originalDebugPrint = debugPrint;
        debugPrint = (message, {wrapWidth}) {
          if (message != null) logs.add(message);
        };
        final container = _buildContainer(session: session);
        container.read(channelsProvider);
        try {
          clock.flushMicrotasks();
          // The finite snapshot is usable even before live readiness settles.
          expect(
            container.read(channelsProvider).requireValue.single.id,
            _channelA,
          );
          expect(session.totalSubscribeCount, 1);
          final initialMemberships = session.membershipRequestCount;
          clock.elapse(const Duration(milliseconds: 2));
          final historyAfterClose = session.historyFilters.length;
          final batchesAfterClose = session.queryBatches.length;
          final logsAfterClose = logs.length;

          // Virtual time permits many immediate retries on the broken path.
          for (var i = 0; i < 20; i++) {
            clock.elapse(const Duration(milliseconds: 10));
          }
          expect(session.rejectionCount, 1);
          expect(session.membershipRequestCount, initialMemberships);
          expect(session.historyFilters.length, historyAfterClose);
          expect(session.queryBatches.length, batchesAfterClose);
          expect(session.totalSubscribeCount, 1);
          expect(logs.length, logsAfterClose);
          expect(clock.periodicTimerCount, 1);
          expect(clock.nonPeriodicTimerCount, 0);

          // Ordinary polling may refresh memberships, but never re-admits an
          // unchanged terminal filter, creates retry timers, or repeats logs.
          final pollStart = session.membershipRequestCount;
          for (var i = 0; i < 3; i++) {
            clock.elapse(const Duration(seconds: 60));
            expect(session.totalSubscribeCount, 1);
            expect(session.activeSubscriptionCount, 0);
            expect(logs.length, logsAfterClose);
            expect(clock.periodicTimerCount, 1);
            expect(clock.nonPeriodicTimerCount, 0);
            expect(
              container.read(channelsProvider).requireValue.single.id,
              _channelA,
            );
          }
          expect(session.membershipRequestCount - pollStart, 6);

          // A pull-to-refresh still isn't evidence that admission changed.
          unawaited(container.read(channelsProvider.notifier).refresh());
          clock.flushMicrotasks();
          clock.elapse(const Duration(milliseconds: 2));
          expect(session.totalSubscribeCount, 1);
          expect(logs.length, logsAfterClose);
        } finally {
          container.dispose();
          debugPrint = originalDebugPrint;
        }
        clock.elapse(const Duration(seconds: 60));
        expect(clock.pendingTimers, isEmpty);
        expect(session.totalSubscribeCount, 1);
      },
    );
  }

  for (final beforeReady in [true, false]) {
    _testWithClock(
      'disposal fences pending terminal closure ${beforeReady ? 'before' : 'after'} readiness',
      (clock) {
        final session = _TerminalRelaySession(beforeReady: beforeReady);
        final container = _buildContainer(session: session);
        container.read(channelsProvider);
        clock.flushMicrotasks();
        final requests = session.membershipRequestCount;
        container.dispose();
        clock.elapse(const Duration(minutes: 3));
        expect(session.totalSubscribeCount, 1);
        expect(session.membershipRequestCount, requests);
        expect(session.activeSubscriptionCount, 0);
        expect(clock.pendingTimers, isEmpty);
      },
    );
  }

  _testWithClock('terminal rejection retries only after a new connection', (
    clock,
  ) {
    final session = _TerminalRelaySession();
    final container = _buildContainer(session: session);
    try {
      container.read(channelsProvider);
      clock.flushMicrotasks();
      clock.elapse(const Duration(milliseconds: 2));
      final oldClosed = session.closedCallbacks.single;
      for (var attempt = 2; attempt <= 4; attempt++) {
        session.setStatus(SessionStatus.disconnected);
        clock.flushMicrotasks();
        session.setStatus(SessionStatus.connected);
        clock.flushMicrotasks();
        clock.elapse(const Duration(milliseconds: 2));
        clock.elapse(const Duration(milliseconds: 20));
        expect(session.totalSubscribeCount, attempt);
        expect(session.rejectionCount, attempt);
        expect(session.activeSubscriptionCount, 0);
        expect(
          container.read(channelsProvider).requireValue.single.id,
          _channelA,
        );
      }
      session.reject = false;
      session.setStatus(SessionStatus.disconnected);
      clock.flushMicrotasks();
      session.setStatus(SessionStatus.connected);
      clock.flushMicrotasks();
      expect(session.totalSubscribeCount, 5);
      expect(session.activeChannels, {_channelA});
      oldClosed();
      clock.flushMicrotasks();
      expect(session.activeChannels, {_channelA});
      expect(session.totalSubscribeCount, 5);
    } finally {
      container.dispose();
    }
    clock.flushMicrotasks();
  });

  _testWithClock(
    'changed membership admits a new filter, not a rejected old one',
    (clock) {
      final session = _TerminalRelaySession();
      final container = _buildContainer(session: session);
      try {
        container.read(channelsProvider);
        clock.flushMicrotasks();
        clock.elapse(const Duration(milliseconds: 2));
        session.reject = false;
        session.memberships.add(_membership(_channelB, 'me'));
        session.metadata.add(_meta(id: _channelB, name: 'b'));
        unawaited(container.read(channelsProvider.notifier).refresh());
        clock.flushMicrotasks();
        expect(session.totalSubscribeCount, 2);
        expect(session.activeChannels, {_channelA, _channelB});
        // Removing then rejoining is a meaningful transition even if it returns
        // to an earlier filter; obsolete quarantine keys must not leak forever.
        session.memberships.removeLast();
        unawaited(container.read(channelsProvider.notifier).refresh());
        clock.flushMicrotasks();
        expect(session.totalSubscribeCount, 3);
        expect(session.activeChannels, {_channelA});
      } finally {
        container.dispose();
      }
      clock.flushMicrotasks();
    },
  );

  for (final switchIdentity in [true, false]) {
    _testWithClock(
      'terminal quarantine does not cross ${switchIdentity ? 'identity' : 'community'} scope',
      (clock) {
        final session = _TerminalRelaySession();
        final container = _buildContainer(session: session);
        try {
          container.read(channelsProvider);
          clock.flushMicrotasks();
          clock.elapse(const Duration(milliseconds: 2));
          session.reject = false;
          if (switchIdentity) {
            session.memberships = [_membership(_channelA, _otherPk)];
            container.read(_testPubkeyProvider.notifier).set(_otherPk);
          } else {
            container
                .read(relayConfigProvider.notifier)
                .update(baseUrl: 'https://new-community.example');
          }
          container.read(channelsProvider);
          clock.flushMicrotasks();
          expect(session.totalSubscribeCount, 2);
          expect(session.activeChannels, {_channelA});
        } finally {
          container.dispose();
        }
        clock.flushMicrotasks();
      },
    );
  }
}

/// Reject every attempted subscription, including every replacement, as a
/// terminal relay CLOSED. A timer models the wire turn and prevents a broken
/// immediate retry loop from hanging the test's microtask drain.
class _TerminalRelaySession extends _FakeRelaySession {
  _TerminalRelaySession({this.beforeReady = false})
    : super(
        memberships: [_membership(_channelA, 'me')],
        metadata: [_meta(id: _channelA, name: 'a')],
      );

  final bool beforeReady;
  bool reject = true;
  int rejectionCount = 0;
  final List<void Function()> closedCallbacks = [];

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    final unsubscribe = await super.subscribe(
      filter,
      onEvent,
      onClosed: onClosed,
    );
    if (!reject) return unsubscribe;
    final key = _subscriptions.keys.last;
    final closed = Completer<void>();
    final timer = Timer(const Duration(milliseconds: 1), () {
      final subscription = _subscriptions.remove(key);
      if (subscription != null) {
        subscribeFilters.remove(subscription.$1);
        rejectionCount++;
        void notify() => onClosed?.call('restricted: persistent rejection');
        closedCallbacks.add(notify);
        notify();
      }
      closed.complete();
    });
    if (beforeReady) {
      await closed.future;
      throw StateError('restricted: persistent rejection');
    }
    return () {
      timer.cancel();
      unsubscribe();
    };
  }
}
