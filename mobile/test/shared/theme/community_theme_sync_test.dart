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

  test(
    'published coordinate rejects delayed same-second initialization result',
    () async {
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
      final createdAt = relay.submittedEvents.single.createdAt;
      history.complete([
        _event(
          id: 'published-z',
          createdAt: createdAt,
          content: jsonEncode(stale.toJson()),
        ),
      ]);
      await initializing;

      expect(applied, isEmpty);
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
    this.onFetchHistory,
  }) : history = List.of(history);
  List<NostrEvent> history;
  final Future<List<NostrEvent>>? historyFuture;
  final Object? error;
  final void Function()? onFetchHistory;
  int subscribeCalls = 0;
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
    if (error != null) throw error!;
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
  });
  int failuresRemaining;
  final String eventId;
  final Future<void>? firstSubmissionGate;
  int attempts = 0;
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
    return event;
  }
}
