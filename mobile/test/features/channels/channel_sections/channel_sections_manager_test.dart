import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_manager.dart';
import 'package:buzz/shared/relay/relay.dart';

void main() {
  late SharedPreferences prefs;
  late nostr.Keys keychain;
  late ChannelSectionsCrypto crypto;

  Future<void> setUpEnv() async {
    SharedPreferences.setMockInitialValues({});
    prefs = await SharedPreferences.getInstance();
    keychain = nostr.Keys.generate();
    crypto = ChannelSectionsCrypto(keychain.nsec, keychain.public);
  }

  NostrEvent sectionsEvent({
    required List<Map<String, dynamic>> sections,
    Map<String, String> assignments = const {},
    required int createdAt,
    String id = 'remote-event',
  }) {
    final payload = jsonEncode({
      'version': 1,
      'sections': sections,
      'assignments': assignments,
    });
    return NostrEvent(
      id: id,
      pubkey: keychain.public,
      createdAt: createdAt,
      kind: EventKind.readState,
      tags: const [
        ['d', 'channel-sections'],
        ['t', 'channel-sections'],
      ],
      content: crypto.encrypt(payload),
      sig: 'sig',
    );
  }

  ChannelSectionsManager buildManager({
    required RelaySessionNotifier relaySession,
    Duration startupRetryBaseDelay = const Duration(milliseconds: 5),
  }) {
    return ChannelSectionsManager(
      pubkey: keychain.public,
      prefs: prefs,
      crypto: crypto,
      relaySession: relaySession,
      signedEventRelay: null,
      remoteEnabled: true,
      onChanged: () {},
      startupRetryBaseDelay: startupRetryBaseDelay,
    );
  }

  test('startup fetch rejected by relay rate limit retries until the remote '
      'blob is adopted (cold-start regression)', () async {
    await setUpEnv();
    // Cold start: the relay rejects the first fetch AND the first live
    // subscription with `rate-limited: quota exceeded` because the channel
    // list fired dozens of REQs first. Pre-fix, both errors were swallowed
    // and desktop-created groups never appeared until app data was cleared.
    final relay = _RateLimitedRelaySession(
      failuresBeforeSuccess: 2,
      historyEvents: [
        sectionsEvent(
          sections: [
            {'id': 's1', 'name': 'Desktop Group', 'order': 0},
          ],
          createdAt: 100,
        ),
      ],
    );

    final manager = buildManager(
      relaySession: relay,
      startupRetryBaseDelay: const Duration(milliseconds: 10),
    );
    await manager.initialize();

    expect(
      manager.store.sections,
      isEmpty,
      reason: 'first fetch lost the rate-limit race',
    );

    // Wait for the backoff retries (10ms, 20ms, …) to win the race.
    await _waitUntil(() => manager.store.sections.isNotEmpty);

    expect(manager.store.sections.single.name, 'Desktop Group');
    expect(
      relay.subscribeCalls,
      greaterThan(1),
      reason: 'live subscription must be retried too',
    );
    manager.dispose(flushPending: false);
  });

  test(
    'startup retry stops after fetch and subscription both succeed',
    () async {
      await setUpEnv();
      final relay = _RateLimitedRelaySession(
        failuresBeforeSuccess: 1,
        historyEvents: [
          sectionsEvent(
            sections: [
              {'id': 's1', 'name': 'Desktop Group', 'order': 0},
            ],
            createdAt: 100,
          ),
        ],
      );
      final manager = buildManager(relaySession: relay);
      await manager.initialize();
      await _waitUntil(() => manager.store.sections.isNotEmpty);

      final fetchCallsAfterSuccess = relay.fetchCalls;
      final subscribeCallsAfterSuccess = relay.subscribeCalls;
      await Future<void>.delayed(const Duration(milliseconds: 60));

      expect(relay.fetchCalls, fetchCallsAfterSuccess);
      expect(relay.subscribeCalls, subscribeCallsAfterSuccess);
      manager.dispose(flushPending: false);
    },
  );

  test('disposing the manager cancels pending startup retries', () async {
    await setUpEnv();
    final relay = _RateLimitedRelaySession(failuresBeforeSuccess: 1000);
    final manager = buildManager(relaySession: relay);
    await manager.initialize();
    manager.dispose(flushPending: false);

    final fetchCallsAtDispose = relay.fetchCalls;
    await Future<void>.delayed(const Duration(milliseconds: 60));

    expect(
      relay.fetchCalls,
      fetchCallsAtDispose,
      reason: 'no retries may fire after dispose',
    );
  });

  test('late relay CLOSED after subscribe() reported success retries and '
      'eventually adopts remote state', () async {
    await setUpEnv();
    // Under cold-start load, subscribe() can time out its 500ms readiness
    // wait and resolve successfully before the relay's rate-limit CLOSED
    // lands. The rejection then arrives only via onClosed. Pre-fix the
    // manager passed no onClosed, kept the dead subscription, and never
    // retried.
    final relay = _RateLimitedRelaySession(failuresBeforeSuccess: 0);
    final manager = buildManager(relaySession: relay);
    await manager.initialize();
    expect(relay.subscribeCalls, 1);

    // Late CLOSED lands after initialize() completed successfully.
    relay.closeLiveSubscription('rate-limited: quota exceeded');

    // The manager must drop the dead subscription and re-subscribe.
    await _waitUntil(() => relay.subscribeCalls > 1);

    // Remote state arriving on the NEW subscription must be adopted.
    relay.emit(
      sectionsEvent(
        sections: [
          {'id': 's1', 'name': 'Desktop Group', 'order': 0},
        ],
        createdAt: 100,
      ),
    );
    await _waitUntil(() => manager.store.sections.isNotEmpty);
    expect(manager.store.sections.single.name, 'Desktop Group');

    // Exactly one replacement subscription — no duplicates.
    await Future<void>.delayed(const Duration(milliseconds: 60));
    expect(relay.subscribeCalls, 2);
    expect(relay.activeListeners, 1);
    manager.dispose(flushPending: false);
  });

  test('late CLOSED after dispose does not schedule retries', () async {
    await setUpEnv();
    final relay = _RateLimitedRelaySession(failuresBeforeSuccess: 0);
    final manager = buildManager(relaySession: relay);
    await manager.initialize();
    manager.dispose(flushPending: false);

    relay.closeLiveSubscription('rate-limited: quota exceeded');
    await Future<void>.delayed(const Duration(milliseconds: 60));

    expect(relay.subscribeCalls, 1, reason: 'no re-subscribe after dispose');
  });

  test(
    'dispose while subscribe is pending closes the late subscription',
    () async {
      await setUpEnv();
      final relay = _DelayedSubscribeRelaySession();
      final manager = buildManager(relaySession: relay);

      final initializing = manager.initialize();
      await relay.subscribeStarted.future;
      manager.dispose(flushPending: false);
      relay.completeSubscribe();
      await initializing;

      expect(relay.activeListeners, 0);
      expect(relay.unsubscribeCalls, 1);
    },
  );

  test(
    'a retry request during an in-flight sync does not overlap subscribe',
    () async {
      await setUpEnv();
      final relay = _DelayedSubscribeRelaySession();
      final manager = buildManager(relaySession: relay);

      final initializing = manager.initialize();
      await relay.subscribeStarted.future;
      relay.closePendingSubscription('rate-limited: quota exceeded');
      await Future<void>.delayed(const Duration(milliseconds: 20));

      expect(relay.subscribeCalls, 1);
      relay.completeSubscribe();
      await initializing;
      await _waitUntil(() => relay.subscribeCalls == 2);
      expect(relay.maxConcurrentSubscribes, 1);
      expect(relay.activeListeners, 1);
      manager.dispose(flushPending: false);
    },
  );

  test('backoff resets after full recovery so later failures start from the '
      'base delay', () async {
    await setUpEnv();
    // Climb the failure counter: 5 rate-limited attempts before success.
    // With a 5ms base, attempt 5 would wait 5<<5 = 160ms if the counter
    // were never reset.
    final relay = _RateLimitedRelaySession(failuresBeforeSuccess: 5);
    final manager = buildManager(relaySession: relay);
    await manager.initialize();
    await _waitUntil(() => relay.subscribeCalls > 5);

    // Fully recovered. A later transient failure (late CLOSED) must retry
    // from the base delay, not the climbed ceiling.
    final subscribeCallsAfterRecovery = relay.subscribeCalls;
    relay.closeLiveSubscription('rate-limited: quota exceeded');
    final stopwatch = Stopwatch()..start();
    await _waitUntil(() => relay.subscribeCalls > subscribeCallsAfterRecovery);
    stopwatch.stop();

    expect(
      stopwatch.elapsedMilliseconds,
      lessThan(100),
      reason:
          'retry after recovery must wait ~base delay (5ms), '
          'not the pre-recovery backoff (160ms)',
    );
    manager.dispose(flushPending: false);
  });
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
    await Future<void>.delayed(const Duration(milliseconds: 10));
  }
}

class _DelayedSubscribeRelaySession extends RelaySessionNotifier {
  final subscribeStarted = Completer<void>();
  Completer<void Function()>? _pendingSubscribe;
  void Function(String)? _pendingOnClosed;
  int subscribeCalls = 0;
  int concurrentSubscribes = 0;
  int maxConcurrentSubscribes = 0;
  int activeListeners = 0;
  int unsubscribeCalls = 0;

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => const [];

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) {
    subscribeCalls++;
    concurrentSubscribes++;
    if (concurrentSubscribes > maxConcurrentSubscribes) {
      maxConcurrentSubscribes = concurrentSubscribes;
    }
    if (!subscribeStarted.isCompleted) subscribeStarted.complete();
    _pendingOnClosed = onClosed;
    if (subscribeCalls > 1) {
      concurrentSubscribes--;
      activeListeners++;
      return Future.value(_unsubscribe);
    }
    _pendingSubscribe = Completer<void Function()>();
    return _pendingSubscribe!.future.whenComplete(() {
      concurrentSubscribes--;
    });
  }

  void closePendingSubscription(String message) =>
      _pendingOnClosed?.call(message);

  void completeSubscribe() {
    activeListeners++;
    _pendingSubscribe!.complete(_unsubscribe);
  }

  void _unsubscribe() {
    activeListeners--;
    unsubscribeCalls++;
  }
}

/// Rejects the first [failuresBeforeSuccess] fetch and subscribe calls with
/// the relay's rate-limit error, then succeeds — the exact failure mode seen
/// on Android cold start where the channel-list REQ burst exhausts the
/// relay's per-connection quota before the sections manager gets a turn.
class _RateLimitedRelaySession extends RelaySessionNotifier {
  _RateLimitedRelaySession({
    required this.failuresBeforeSuccess,
    this.historyEvents = const [],
  });

  final int failuresBeforeSuccess;
  final List<NostrEvent> historyEvents;
  int fetchCalls = 0;
  int subscribeCalls = 0;
  final List<void Function(NostrEvent)> _listeners = [];
  final List<void Function(String)> _closedCallbacks = [];

  int get activeListeners => _listeners.length;

  void emit(NostrEvent event) {
    for (final listener in List.of(_listeners)) {
      listener(event);
    }
  }

  /// Simulates a relay CLOSED landing after subscribe() already resolved —
  /// the readiness-timeout window on a loaded cold start. Drops the live
  /// subscription (as _handleClosed does) and invokes onClosed.
  void closeLiveSubscription(String message) {
    if (_listeners.isEmpty) return;
    _listeners.removeAt(0);
    final onClosed = _closedCallbacks.removeAt(0);
    onClosed(message);
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    fetchCalls++;
    if (fetchCalls <= failuresBeforeSuccess) {
      throw Exception('rate-limited: quota exceeded; retry in 2s');
    }
    return historyEvents;
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    subscribeCalls++;
    if (subscribeCalls <= failuresBeforeSuccess) {
      throw Exception('rate-limited: quota exceeded; retry in 1s');
    }
    _listeners.add(onEvent);
    _closedCallbacks.add(onClosed ?? (_) {});
    return () {
      final index = _listeners.indexOf(onEvent);
      if (index >= 0) {
        _listeners.removeAt(index);
        _closedCallbacks.removeAt(index);
      }
    };
  }
}
