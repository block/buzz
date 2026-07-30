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

    final result = await manager.initialize(local);
    expect(result.status, CommunityThemeRemoteStatus.absent);
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

  test('invalid and unavailable records never seed', () async {
    for (final session in [
      _FakeSession(history: [_event(content: '{bad json')]),
      _FakeSession(error: StateError('offline')),
    ]) {
      final relay = _FakeSignedRelay();
      final result = await _manager(session, relay).initialize(local);
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
          _event(id: 'a', createdAt: 50, content: jsonEncode(local.toJson())),
          _event(
            id: 'b',
            createdAt: 50,
            content: jsonEncode(
              const CommunityThemePreference(
                theme: 'dracula',
                accent: '#ef4444',
                followSystem: false,
              ).toJson(),
            ),
          ),
        ],
      );
      final manager = _manager(
        session,
        _FakeSignedRelay(),
        onRemote: (r) => applied.add(r.preference),
      );

      await manager.initialize(local);
      expect(applied.single.theme, 'dracula');

      session.emit(
        _event(id: 'aa', createdAt: 50, content: jsonEncode(local.toJson())),
      );
      expect(applied, hasLength(1));
    },
  );

  test(
    'remote apply cancels pending user write and dispose guards scope',
    () async {
      final relay = _FakeSignedRelay();
      final session = _FakeSession();
      final manager = _manager(session, relay);
      await manager.initialize(local);
      manager.cancelPending();
      manager.publish(
        const CommunityThemePreference(
          theme: 'dracula',
          accent: '#ef4444',
          followSystem: false,
        ),
      );

      session.emit(
        _event(
          id: 'remote',
          createdAt: 100,
          content: jsonEncode(local.toJson()),
        ),
      );
      await manager.flush();
      expect(relay.submissions, isEmpty);

      manager.publish(local);
      manager.dispose();
      await manager.flush();
      expect(relay.submissions, isEmpty);
    },
  );

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
      await manager.initialize(local);
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
    await manager.initialize(local);
    final close = session.latestClosedCallback;

    manager.dispose();
    close?.call('late close');
    await Future<void>.delayed(const Duration(milliseconds: 20));

    expect(session.subscribeCalls, 1);
  });

  test(
    'publish failure keeps pending preference for reconnect retry',
    () async {
      final relay = _FakeSignedRelay(fail: true);
      final manager = _manager(_FakeSession(), relay);
      manager.publish(local);
      await manager.flush();
      expect(manager.pending, local);
    },
  );
}

CommunityThemeSyncManager _manager(
  _FakeSession session,
  _FakeSignedRelay relay, {
  void Function(RemoteCommunityTheme)? onRemote,
}) => CommunityThemeSyncManager(
  pubkey: 'pk',
  relaySession: session,
  signedEventRelay: relay,
  crypto: const CommunityThemeCrypto(encrypt: _identity, decrypt: _identity),
  debounce: const Duration(days: 1),
  subscriptionRetryBase: const Duration(milliseconds: 1),
  onRemote: onRemote ?? (_) {},
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
  _FakeSession({List<NostrEvent> history = const [], this.error})
    : history = List.of(history);
  List<NostrEvent> history;
  final Object? error;
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
  _FakeSignedRelay({this.fail = false});
  final bool fail;
  final submissions = <_Submission>[];
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
    if (fail) throw StateError('publish failed');
    submissions.add(_Submission(kind, content, tags));
    return _event(content: content, createdAt: createdAt ?? 0);
  }
}
