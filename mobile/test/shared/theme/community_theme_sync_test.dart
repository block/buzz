import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  const local = CommunityThemePreference(
    theme: 'buzz',
    accent: '#3b82f6',
    followSystem: true,
    glassBackground: true,
    glassOpacity: 48,
    prominentActiveTab: false,
  );

  test('confirmed absence seeds exact NIP-78 coordinate', () async {
    final session = _FakeSession();
    final relay = _FakeSignedRelay();
    final manager = _manager(session, relay);

    final result = await manager.initialize();
    expect(result.status, CommunityThemeRemoteStatus.absent);
    manager.publish(local);
    await manager.flush();

    expect(relay.submissions, hasLength(1));
    expect(relay.submissions.single.kind, 30078);
    expect(
      relay.submissions.single.tags,
      containsAll(<List<String>>[
        ['d', 'community-theme'],
        ['t', 'community-theme'],
      ]),
    );
    expect(jsonDecode(relay.submissions.single.content), local.toJson());
  });

  test(
    'live replacement closes the history-to-subscription absence gap',
    () async {
      const replacement = CommunityThemePreference(
        theme: 'dracula',
        accent: '#ef4444',
        followSystem: false,
      );
      final applied = <CommunityThemePreference>[];
      late final _FakeSession session;
      session = _FakeSession(
        onFetchHistory: () {
          session.emit(
            _event(
              id: 'replacement',
              createdAt: 100,
              content: jsonEncode(replacement.toJson()),
            ),
          );
        },
      );
      final manager = _manager(
        session,
        _FakeSignedRelay(),
        onRemote: (remote) => applied.add(remote.preference),
      );

      final result = await manager.initialize();

      expect(session.subscribeCalls, 1);
      expect(result.status, CommunityThemeRemoteStatus.valid);
      expect(result.remote?.preference, replacement);
      expect(applied, [replacement]);
    },
  );

  test('invalid and unavailable records never seed', () async {
    for (final session in [
      _FakeSession(history: [_event(content: '{bad json')]),
      _FakeSession(error: StateError('offline')),
    ]) {
      final relay = _FakeSignedRelay();
      final result = await _manager(session, relay).initialize();
      expect(
        result.status,
        anyOf(
          CommunityThemeRemoteStatus.invalid,
          CommunityThemeRemoteStatus.unavailable,
        ),
      );
      expect(relay.submissions, isEmpty);
    }
  });

  test(
    'newest valid event wins with deterministic same-second ordering',
    () async {
      final applied = <CommunityThemePreference>[];
      final session = _FakeSession(
        history: [
          _event(
            id: 'z',
            createdAt: 50,
            content: jsonEncode(
              const CommunityThemePreference(
                theme: 'dracula',
                accent: '#ef4444',
                followSystem: false,
              ).toJson(),
            ),
          ),
          _event(id: 'a', createdAt: 50, content: jsonEncode(local.toJson())),
        ],
      );
      final manager = _manager(
        session,
        _FakeSignedRelay(),
        onRemote: (r) => applied.add(r.preference),
      );

      await manager.initialize();
      expect(applied.single.theme, 'buzz');

      session.emit(
        _event(
          id: 'z',
          createdAt: 50,
          content: jsonEncode(
            const CommunityThemePreference(
              theme: 'dracula',
              accent: '#ef4444',
              followSystem: false,
            ).toJson(),
          ),
        ),
      );
      expect(applied, hasLength(1));
    },
  );

  test('remote hydration never cancels a newer pending local write', () async {
    final relay = _FakeSignedRelay();
    final session = _FakeSession();
    final manager = _manager(session, relay);
    await manager.initialize();
    manager.cancelPending();
    manager.publish(
      const CommunityThemePreference(
        theme: 'dracula',
        accent: '#ef4444',
        followSystem: false,
      ),
    );

    session.emit(
      _event(id: 'remote', createdAt: 100, content: jsonEncode(local.toJson())),
    );
    await manager.flush();
    expect(relay.submissions, hasLength(1));
    expect(jsonDecode(relay.submissions.single.content)['theme'], 'dracula');

    manager.publish(local);
    manager.dispose();
    await manager.flush();
    expect(relay.submissions, hasLength(1));
  });

  test(
    'relay CLOSED resubscribes then catches up latest replacement event',
    () async {
      final applied = <CommunityThemePreference>[];
      final session = _FakeSession();
      final manager = _manager(
        session,
        _FakeSignedRelay(),
        onRemote: (remote) => applied.add(remote.preference),
      );
      await manager.initialize();
      manager.cancelPending();
      expect(session.subscribeCalls, 1);

      const replacement = CommunityThemePreference(
        theme: 'dracula',
        accent: '#ef4444',
        followSystem: false,
      );
      session.history = [
        _event(
          id: 'replacement',
          createdAt: 100,
          content: jsonEncode(replacement.toJson()),
        ),
      ];
      session.closeLiveSubscription('rate-limited: quota exceeded');

      await _waitUntil(() => session.subscribeCalls == 2 && applied.isNotEmpty);
      expect(applied.single, replacement);
      expect(session.activeListeners, 1);
      manager.dispose();
    },
  );

  test('relay CLOSED after dispose never resubscribes', () async {
    final session = _FakeSession();
    final manager = _manager(session, _FakeSignedRelay());
    await manager.initialize();
    final close = session.latestClosedCallback;

    manager.dispose();
    close?.call('late close');
    await Future<void>.delayed(const Duration(milliseconds: 20));

    expect(session.subscribeCalls, 1);
  });

  test('publish failure retries and acknowledges exact preference', () async {
    final relay = _FakeSignedRelay(failuresRemaining: 1);
    final acknowledgements = <CommunityThemePreference>[];
    final manager = _manager(
      _FakeSession(),
      relay,
      onPublished: acknowledgements.add,
    );
    manager.publish(local);
    await manager.flush();
    expect(manager.pending, local);

    await _waitUntil(() => acknowledgements.length == 1);
    expect(relay.attempts, 2);
    expect(manager.pending, isNull);
    expect(acknowledgements, [local]);
  });

  test('serializes in-flight publish before latest edit', () async {
    final firstSubmission = Completer<void>();
    final relay = _FakeSignedRelay(firstSubmissionGate: firstSubmission.future);
    final manager = _manager(_FakeSession(), relay);
    const latest = CommunityThemePreference(
      theme: 'dracula',
      accent: '#ef4444',
      followSystem: false,
    );

    manager.publish(local);
    final firstFlush = manager.flush();
    await _waitUntil(() => relay.attempts == 1);
    manager.publish(latest);
    await manager.flush();
    expect(relay.attempts, 1);

    firstSubmission.complete();
    await firstFlush;
    await _waitUntil(() => relay.attempts == 2);

    expect(relay.submittedEvents, hasLength(2));
    expect(
      relay.submittedEvents[1].createdAt,
      greaterThan(relay.submittedEvents[0].createdAt),
    );
    expect(jsonDecode(relay.submissions[1].content)['theme'], 'dracula');
    expect(manager.pending, isNull);
  });

  test(
    'republishes above remote observed while publish is in flight',
    () async {
      final firstSubmission = Completer<void>();
      final relay = _FakeSignedRelay(
        firstSubmissionGate: firstSubmission.future,
      );
      final session = _FakeSession();
      final acknowledgements = <CommunityThemePreference>[];
      final manager = _manager(
        session,
        relay,
        onPublished: acknowledgements.add,
      );
      await manager.initialize();
      manager.publish(local);
      final firstFlush = manager.flush();
      await _waitUntil(() => relay.attempts == 1);

      session.emit(
        _event(
          id: 'remote-winner',
          createdAt: 2000000000,
          content: jsonEncode(
            const CommunityThemePreference(
              theme: 'dracula',
              accent: '#ef4444',
              followSystem: false,
            ).toJson(),
          ),
        ),
      );
      firstSubmission.complete();
      await firstFlush;

      expect(acknowledgements, isEmpty);
      expect(manager.pending, local);
      await _waitUntil(() => relay.attempts == 2);
      expect(relay.submittedEvents[1].createdAt, 2000000001);
      expect(acknowledgements, [local]);
      expect(manager.pending, isNull);
    },
  );

  test(
    're-merges desktop fields when remote advances during submission',
    () async {
      final firstSubmission = Completer<void>();
      final session = _FakeSession();
      final relay = _FakeSignedRelay(
        eventId: 'a-mobile',
        firstSubmissionGate: firstSubmission.future,
        onBeforeAcknowledge: session.emit,
      );
      final acknowledgements = <CommunityThemePreference>[];
      final manager = _manager(
        session,
        relay,
        onPublished: acknowledgements.add,
      );
      await manager.initialize();
      manager.publish(local);
      final firstFlush = manager.flush();
      await _waitUntil(() => relay.attempts == 1);

      const desktop = CommunityThemePreference(
        theme: 'dracula',
        accent: '#ef4444',
        followSystem: false,
        glassBackground: false,
        glassOpacity: 80,
        prominentActiveTab: true,
      );
      session.emit(
        _event(
          id: 'z-desktop',
          createdAt: relay.startedCreatedAts.single,
          content: jsonEncode(desktop.toJson()),
        ),
      );
      firstSubmission.complete();
      await firstFlush;

      expect(acknowledgements, isEmpty);
      expect(manager.pending, local);
      await _waitUntil(() => relay.attempts == 2);

      final recovered =
          jsonDecode(relay.submissions[1].content) as Map<String, dynamic>;
      expect(recovered['theme'], local.theme);
      expect(recovered['accent'], local.accent);
      expect(recovered['glassBackground'], false);
      expect(recovered['glassOpacity'], 80);
      expect(recovered['prominentActiveTab'], true);
      expect(
        relay.submittedEvents[1].createdAt,
        greaterThan(relay.startedCreatedAts.first),
      );
      expect(acknowledgements, [local]);
      expect(manager.pending, isNull);
    },
  );

  test(
    'matching self echo before acknowledgement does not republish',
    () async {
      final session = _FakeSession();
      final relay = _FakeSignedRelay(onBeforeAcknowledge: session.emit);
      final acknowledgements = <CommunityThemePreference>[];
      final manager = _manager(
        session,
        relay,
        onPublished: acknowledgements.add,
      );
      await manager.initialize();
      manager.publish(local);

      await manager.flush();
      await Future<void>.delayed(const Duration(milliseconds: 10));

      expect(relay.attempts, 1);
      expect(acknowledgements, [local]);
      expect(manager.pending, isNull);
    },
  );

  test('remote coordinate advances pending local publish timestamp', () async {
    final relay = _FakeSignedRelay();
    final session = _FakeSession();
    final manager = _manager(session, relay);
    await manager.initialize();
    manager.publish(local);

    session.emit(
      _event(
        id: 'remote',
        createdAt: 2000000000,
        content: jsonEncode(local.toJson()),
      ),
    );
    await manager.flush();

    expect(relay.submittedEvents.single.createdAt, 2000000001);
  });

  test(
    'remote replacement invalidates A to B to A no-op suppression',
    () async {
      final relay = _FakeSignedRelay();
      final session = _FakeSession();
      final manager = _manager(session, relay);
      await manager.initialize();
      manager.publish(local);
      await manager.flush();

      const remotePreference = CommunityThemePreference(
        theme: 'dracula',
        accent: '#ef4444',
        followSystem: false,
      );
      session.emit(
        _event(
          id: 'remote',
          createdAt: relay.submittedEvents.single.createdAt + 1,
          content: jsonEncode(remotePreference.toJson()),
        ),
      );
      manager.publish(local);
      await manager.flush();

      expect(relay.submissions, hasLength(2));
      expect(manager.pending, isNull);
    },
  );

  test('a delayed initialization result never overrides a held edit', () async {
    // The gate holds the edit until initialize resolves, so a publish can no
    // longer race ahead of hydration. When the delayed history arrives it is
    // observed (not applied over the pending edit), the gate releases, and
    // the held edit publishes above the observed coordinate.
    const stale = CommunityThemePreference(
      theme: 'dracula',
      accent: '#ef4444',
      followSystem: false,
    );
    final history = Completer<List<NostrEvent>>();
    final session = _FakeSession(historyFuture: history.future);
    final relay = _FakeSignedRelay(eventId: 'published-a');
    final applied = <CommunityThemePreference>[];
    final manager = _manager(
      session,
      relay,
      onRemote: (remote) => applied.add(remote.preference),
    );

    final initializing = manager.initialize();
    manager.publish(local);
    await manager.flush();
    // Held: initialize has not resolved, so the coordinate is unobserved.
    expect(relay.submissions, isEmpty);
    expect(manager.pending, local);

    history.complete([
      _event(
        id: 'published-z',
        createdAt: 100,
        content: jsonEncode(stale.toJson()),
      ),
    ]);
    await initializing;
    await _waitUntil(() => relay.submissions.isNotEmpty);

    // The delayed stale result was observed but never applied over the
    // pending edit, and the released edit still publishes exactly once.
    expect(applied, isEmpty);
    expect(manager.pending, isNull);
    expect(relay.submissions, hasLength(1));
    expect(jsonDecode(relay.submissions.single.content)['theme'], 'buzz');
  });

  test(
    'gated incomplete edit holds until hydration then merges remote glass',
    () async {
      // A desktop client already published glass settings the mobile parser
      // omits. A mobile edit made before hydration must not strip them.
      const desktop = CommunityThemePreference(
        theme: 'buzz',
        accent: '#3b82f6',
        followSystem: true,
        glassBackground: true,
        glassOpacity: 80,
        prominentActiveTab: true,
      );
      const incompleteEdit = CommunityThemePreference(
        theme: 'dracula',
        accent: '#ef4444',
        followSystem: false,
        includesGlassBackground: false,
        includesGlassOpacity: false,
        includesProminentActiveTab: false,
      );
      final history = Completer<List<NostrEvent>>();
      final session = _FakeSession(historyFuture: history.future);
      final relay = _FakeSignedRelay();
      final manager = _manager(session, relay);

      final initializing = manager.initialize();
      // Edit lands before the coordinate is observed: publishing is held.
      manager.publish(incompleteEdit);
      await manager.flush();
      expect(relay.submissions, isEmpty);
      expect(manager.pending, incompleteEdit);

      // Hydration reveals the desktop record; the gate releases and the edit
      // publishes with the desktop-only fields merged back in.
      history.complete([
        _event(
          id: 'desktop',
          createdAt: 100,
          content: jsonEncode(desktop.toJson()),
        ),
      ]);
      await initializing;
      await _waitUntil(() => relay.submissions.isNotEmpty);

      final published =
          jsonDecode(relay.submissions.single.content) as Map<String, dynamic>;
      expect(published['theme'], 'dracula');
      expect(published['glassBackground'], true);
      expect(published['glassOpacity'], 80);
      expect(published['prominentActiveTab'], true);
    },
  );

  test('confirmed absence releases a gated incomplete edit', () async {
    const incompleteEdit = CommunityThemePreference(
      theme: 'dracula',
      accent: '#ef4444',
      followSystem: false,
      includesGlassBackground: false,
      includesGlassOpacity: false,
      includesProminentActiveTab: false,
    );
    final history = Completer<List<NostrEvent>>();
    final session = _FakeSession(historyFuture: history.future);
    final relay = _FakeSignedRelay();
    final manager = _manager(session, relay);

    final initializing = manager.initialize();
    manager.publish(incompleteEdit);
    await manager.flush();
    expect(relay.submissions, isEmpty);

    // No remote record exists; a confirmed absence is enough to release the
    // gate, and with no desktop values to preserve the edit publishes as-is.
    history.complete([]);
    await initializing;
    await _waitUntil(() => relay.submissions.isNotEmpty);

    final published =
        jsonDecode(relay.submissions.single.content) as Map<String, dynamic>;
    expect(published['theme'], 'dracula');
    expect(published.containsKey('glassBackground'), isFalse);
    expect(published.containsKey('glassOpacity'), isFalse);
    expect(published.containsKey('prominentActiveTab'), isFalse);
  });

  test(
    'a full cache edit republishes the observed remote glass, not the cache',
    () async {
      // Regression: a mobile cache decoded from a desktop-authored record
      // carries all three includes* flags, so it looks "complete". Mobile has
      // no UI to author glass, so those fields are stale once another client
      // changes them. The published coordinate must carry the current remote's
      // glass, never the cache's.
      const staleFullCacheEdit = CommunityThemePreference(
        theme: 'dracula',
        accent: '#ef4444',
        followSystem: false,
        glassBackground: true,
        glassOpacity: 80,
        prominentActiveTab: true,
      );
      const currentRemote = CommunityThemePreference(
        theme: 'buzz',
        accent: '#3b82f6',
        followSystem: true,
        glassBackground: false,
        glassOpacity: 40,
        prominentActiveTab: false,
      );
      final session = _FakeSession(
        history: [
          _event(
            id: 'current',
            createdAt: 100,
            content: jsonEncode(currentRemote.toJson()),
          ),
        ],
      );
      final relay = _FakeSignedRelay();
      final manager = _manager(session, relay);

      await manager.initialize();
      manager.publish(staleFullCacheEdit);
      await manager.flush();

      final published =
          jsonDecode(relay.submissions.single.content) as Map<String, dynamic>;
      expect(published['theme'], 'dracula');
      expect(published['accent'], '#ef4444');
      expect(published['glassBackground'], false);
      expect(published['glassOpacity'], 40);
      expect(published['prominentActiveTab'], false);
    },
  );

  test(
    'a transient history failure does not strand a gated edit forever',
    () async {
      // Regression: the pre-hydration gate must not deadlock when the initial
      // fetch fails (unavailable, not absent) while the live subscription stays
      // quiet. A bounded history-retry releases the gate once the relay answers.
      const incompleteEdit = CommunityThemePreference(
        theme: 'dracula',
        accent: '#ef4444',
        followSystem: false,
        includesGlassBackground: false,
        includesGlassOpacity: false,
        includesProminentActiveTab: false,
      );
      // First fetch throws (unavailable); the retry succeeds with an absence.
      final session = _FakeSession(fetchFailuresRemaining: 1);
      final relay = _FakeSignedRelay();
      final manager = _manager(session, relay);

      final result = await manager.initialize();
      expect(result.status, CommunityThemeRemoteStatus.unavailable);

      manager.publish(incompleteEdit);
      await manager.flush();
      // Held: the coordinate has not been observed yet.
      expect(relay.submissions, isEmpty);
      expect(manager.pending, incompleteEdit);

      // The scheduled recovery re-queries history, observes the absence, and
      // releases the gate so the edit finally publishes.
      await _waitUntil(() => relay.submissions.isNotEmpty);
      expect(session.fetchCalls, greaterThanOrEqualTo(2));
      final published =
          jsonDecode(relay.submissions.single.content) as Map<String, dynamic>;
      expect(published['theme'], 'dracula');
      expect(manager.pending, isNull);
    },
  );
}

CommunityThemeSyncManager _manager(
  _FakeSession session,
  _FakeSignedRelay relay, {
  void Function(RemoteCommunityTheme)? onRemote,
  void Function(CommunityThemePreference)? onPublished,
}) => CommunityThemeSyncManager(
  pubkey: 'pk',
  relaySession: session,
  signedEventRelay: relay,
  crypto: const CommunityThemeCrypto(encrypt: _identity, decrypt: _identity),
  debounce: const Duration(days: 1),
  publishRetryBase: const Duration(milliseconds: 1),
  publishRetryMax: const Duration(milliseconds: 4),
  subscriptionRetryBase: const Duration(milliseconds: 1),
  onRemote: onRemote ?? (_) {},
  onPublished: onPublished ?? (_) {},
);

String _identity(String value) => value;

NostrEvent _event({
  String id = 'event',
  int createdAt = 1,
  required String content,
}) => NostrEvent(
  id: id,
  pubkey: 'pk',
  createdAt: createdAt,
  kind: 30078,
  tags: const [
    ['d', 'community-theme'],
  ],
  content: content,
  sig: 'sig',
);

class _FakeSession extends RelaySessionNotifier {
  _FakeSession({
    List<NostrEvent> history = const [],
    this.historyFuture,
    this.error,
    this.fetchFailuresRemaining = 0,
    this.onFetchHistory,
  }) : history = List.of(history);
  List<NostrEvent> history;
  final Future<List<NostrEvent>>? historyFuture;
  final Object? error;
  int fetchFailuresRemaining;
  final void Function()? onFetchHistory;
  int subscribeCalls = 0;
  int fetchCalls = 0;
  final List<void Function(NostrEvent)> _listeners = [];
  final List<void Function(String)> _closedCallbacks = [];

  int get activeListeners => _listeners.length;
  void Function(String)? get latestClosedCallback =>
      _closedCallbacks.isEmpty ? null : _closedCallbacks.last;

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    fetchCalls++;
    if (error != null) throw error!;
    if (fetchFailuresRemaining > 0) {
      fetchFailuresRemaining--;
      throw StateError('history unavailable');
    }
    onFetchHistory?.call();
    if (historyFuture != null) return historyFuture!;
    return history;
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String)? onClosed,
  }) async {
    subscribeCalls++;
    _listeners.add(onEvent);
    _closedCallbacks.add(onClosed ?? (_) {});
    return () {
      final index = _listeners.indexOf(onEvent);
      if (index < 0) return;
      _listeners.removeAt(index);
      _closedCallbacks.removeAt(index);
    };
  }

  void emit(NostrEvent event) {
    for (final listener in List.of(_listeners)) {
      listener(event);
    }
  }

  void closeLiveSubscription(String message) {
    if (_listeners.isEmpty) return;
    _listeners.removeAt(0);
    _closedCallbacks.removeAt(0)(message);
  }
}

Future<void> _waitUntil(
  bool Function() condition, {
  Duration timeout = const Duration(seconds: 2),
}) async {
  final deadline = DateTime.now().add(timeout);
  while (!condition()) {
    if (DateTime.now().isAfter(deadline)) {
      fail('condition not met within $timeout');
    }
    await Future<void>.delayed(const Duration(milliseconds: 1));
  }
}

class _Submission {
  final int kind;
  final String content;
  final List<List<String>> tags;
  const _Submission(this.kind, this.content, this.tags);
}

class _FakeSignedRelay implements SignedEventRelay {
  _FakeSignedRelay({
    this.failuresRemaining = 0,
    this.eventId = 'event',
    this.firstSubmissionGate,
    this.onBeforeAcknowledge,
  });
  int failuresRemaining;
  final String eventId;
  final Future<void>? firstSubmissionGate;
  final void Function(NostrEvent)? onBeforeAcknowledge;
  int attempts = 0;
  final startedCreatedAts = <int>[];
  final submissions = <_Submission>[];
  final submittedEvents = <NostrEvent>[];
  @override
  String? get pubkey => 'pk';
  @override
  Future<NostrEvent> submit({
    required int kind,
    required String content,
    required List<List<String>> tags,
    int? createdAt,
    void Function(NostrEvent)? onSigned,
  }) async {
    attempts++;
    startedCreatedAts.add(createdAt ?? 0);
    if (attempts == 1 && firstSubmissionGate != null) {
      await firstSubmissionGate;
    }
    if (failuresRemaining > 0) {
      failuresRemaining--;
      throw StateError('publish failed');
    }
    submissions.add(_Submission(kind, content, tags));
    final event = _event(
      id: eventId,
      content: content,
      createdAt: createdAt ?? 0,
    );
    onSigned?.call(event);
    submittedEvents.add(event);
    onBeforeAcknowledge?.call(event);
    return event;
  }
}
