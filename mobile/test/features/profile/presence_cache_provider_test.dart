import 'dart:async';

import 'package:fake_async/fake_async.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/profile/presence_cache_provider.dart';
import 'package:buzz/shared/relay/relay.dart';

/// Live-stream behavior alongside the authenticated snapshot backstop.
void main() {
  test('WS presence event updates cache for tracked pubkey', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    // Initialize the notifier (triggers build → subscribes to WS).
    container.read(presenceCacheProvider);
    await _pumpEventQueue();

    // Track alice, then emit her initial 'online' status.
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpEventQueue();
    relaySession.emit(_presence('alice', 'online'));
    expect(container.read(presenceCacheProvider)['alice'], 'online');

    // Simulate a WS presence event: alice goes away.
    relaySession.emit(_presence('alice', 'away'));
    expect(container.read(presenceCacheProvider)['alice'], 'away');
  });

  test('WS presence event ignores untracked pubkeys', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();

    // Track only alice.
    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpEventQueue();

    // Emit event for bob (untracked).
    relaySession.emit(_presence('bob', 'online'));

    // Bob should NOT appear in the cache.
    expect(container.read(presenceCacheProvider).containsKey('bob'), isFalse);
  });

  test('WS presence event ignores invalid status values', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();

    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpEventQueue();
    relaySession.emit(_presence('alice', 'online'));
    expect(container.read(presenceCacheProvider)['alice'], 'online');

    // Emit event with garbage status — should be rejected.
    relaySession.emit(_presence('alice', 'garbage-status'));

    // Status should remain 'online'.
    expect(container.read(presenceCacheProvider)['alice'], 'online');
  });

  test('WS presence event skips no-op updates', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();

    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpEventQueue();
    relaySession.emit(_presence('alice', 'online'));

    // Listen for state changes after initial setup.
    var stateChangeCount = 0;
    container.listen(presenceCacheProvider, (prev, next) => stateChangeCount++);

    // Emit event with same status as current.
    relaySession.emit(_presence('alice', 'online'));

    // No state change should occur — it's a no-op.
    expect(stateChangeCount, 0);
  });

  test('subscribes to kind:20001 for tracked pubkeys only', () async {
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();
    // Nothing is tracked, so nothing is worth a community-wide stream.
    expect(relaySession.filters, isEmpty);

    container.read(presenceCacheProvider.notifier).track(['alice']);
    await _pumpEventQueue();
    expect(relaySession.filters, hasLength(1));
    expect(relaySession.filters.single.kinds, [EventKind.presenceUpdate]);
    expect(relaySession.filters.single.authors, ['alice']);
    expect(relaySession.filters.single.limit, 0);

    // A newly tracked key replaces the subscription with a wider one.
    container.read(presenceCacheProvider.notifier).track(['bob', 'alice']);
    await _pumpEventQueue();
    expect(relaySession.filters, hasLength(1));
    expect(relaySession.filters.single.authors, ['alice', 'bob']);
    relaySession.emit(_presence('bob', 'online'));
    expect(container.read(presenceCacheProvider)['bob'], 'online');
  });

  test(
    'a first subscription still retrying when it opens waits for ready',
    () async {
      final relaySession = _RecordingRelaySessionNotifier()..autoReady = false;
      final container = _buildContainer(relaySession: relaySession);
      addTearDown(container.dispose);

      container.read(presenceCacheProvider);
      container.read(presenceCacheProvider.notifier).track(['alice']);
      await _pumpEventQueue();
      final subscription = relaySession.subscriptions.single;
      // Bob is tracked while alice's first subscription is still opening.
      container.read(presenceCacheProvider.notifier).track(['bob']);
      await _pumpEventQueue();
      expect(relaySession.subscriptions, [subscription]);

      // A retryable CLOSED releases the open before the stream is live, so a
      // snapshot now could miss a change before the retry's EOSE.
      relaySession.snapshot['alice'] = 'online';
      subscription.retrying();
      await _pumpEventQueue();
      expect(relaySession.queried, isEmpty);

      // Its recovery reads both and widens to cover bob live.
      subscription.ready();
      await _pumpEventQueue();
      expect(
        relaySession.queried.expand((keys) => keys),
        containsAll(['alice', 'bob']),
      );
      expect(relaySession.subscriptions.last.filter.authors, ['alice', 'bob']);
      expect(container.read(presenceCacheProvider)['alice'], 'online');
    },
  );

  test('the poll recovers from a replacement the relay never answers', () {
    fakeAsync((time) {
      final relaySession = _RecordingRelaySessionNotifier();
      final container = _buildContainer(relaySession: relaySession);
      addTearDown(container.dispose);
      final notifier = container.read(presenceCacheProvider.notifier);

      container.read(presenceCacheProvider);
      notifier.track(['alice']);
      time.elapse(Duration.zero);
      final first = relaySession.subscriptions.single;

      // Bob's replacement opens on the timeout and never gets EOSE or CLOSED.
      relaySession.autoReady = false;
      notifier.track(['bob']);
      time.elapse(Duration.zero);
      final lost = relaySession.subscriptions.last;
      lost.open();
      time.elapse(Duration.zero);

      // The next poll drops it and widens again; alice's stream stays.
      time.elapse(const Duration(seconds: 60));
      expect(relaySession.subscriptions, hasLength(2));
      expect(relaySession.subscriptions, isNot(contains(lost)));
      final retry = relaySession.subscriptions.last;
      expect(retry.filter.authors, ['alice', 'bob']);
      retry.open();
      time.elapse(Duration.zero);

      // That one is lost too, and then alice's stream closes for good.
      first.closeTerminal();
      time.elapse(Duration.zero);
      expect(relaySession.subscriptions, [retry]);

      // One poll later a fresh subscription covers everyone and reads them.
      relaySession.autoReady = true;
      relaySession.queried.clear();
      time.elapse(const Duration(seconds: 60));
      expect(relaySession.subscriptions, hasLength(1));
      expect(relaySession.subscriptions.single.filter.authors, [
        'alice',
        'bob',
      ]);
      expect(
        relaySession.queried.expand((keys) => keys),
        containsAll(['alice', 'bob']),
      );
      expect(container.read(presenceCacheProvider), {
        'alice': 'offline',
        'bob': 'offline',
      });
    });
  });

  test('the poll still reads a live stream while its widening fails', () {
    fakeAsync((time) {
      final relaySession = _RecordingRelaySessionNotifier();
      final container = _buildContainer(relaySession: relaySession);
      addTearDown(container.dispose);
      final notifier = container.read(presenceCacheProvider.notifier);

      container.read(presenceCacheProvider);
      notifier.track(['alice']);
      time.elapse(Duration.zero);
      relaySession.emit(_presence('alice', 'online'));
      relaySession.autoReady = false;
      notifier.track(['bob']);
      time.elapse(Duration.zero);
      relaySession.subscriptions.last.retrying();
      time.elapse(Duration.zero);
      expect(relaySession.subscriptions, hasLength(1));

      // Alice's lease expired; each poll retries the widening and still
      // reads her, so she is corrected to offline.
      relaySession.queried.clear();
      time.elapse(const Duration(seconds: 60));
      expect(relaySession.subscriptions, hasLength(2));
      expect(relaySession.queried.expand((keys) => keys), contains('alice'));
      expect(container.read(presenceCacheProvider)['alice'], 'offline');
    });
  });

  test(
    'a wider subscription replaces the live one only once it is ready',
    () async {
      final relaySession = _RecordingRelaySessionNotifier();
      final container = _buildContainer(relaySession: relaySession);
      addTearDown(container.dispose);
      final notifier = container.read(presenceCacheProvider.notifier);
      Map<String, String> cache() => container.read(presenceCacheProvider);

      container.read(presenceCacheProvider);
      notifier.track(['alice']);
      await _pumpEventQueue();
      relaySession.emit(_presence('alice', 'online'));
      expect(cache(), {'alice': 'online'});
      final first = relaySession.subscriptions.single;

      // Tracking bob opens a replacement; alice's stream keeps delivering.
      relaySession.autoReady = false;
      notifier.track(['bob']);
      await _pumpEventQueue();
      final retried = relaySession.subscriptions.last;
      expect(retried.filter.authors, ['alice', 'bob']);
      relaySession.emit(_presence('alice', 'away'));
      expect(cache()['alice'], 'away');

      // A second track while one is pending does not open a third.
      notifier.track(['carol']);
      await _pumpEventQueue();
      expect(relaySession.subscriptions, [first, retried]);

      // A retryable CLOSED on the replacement retires it, not alice's stream,
      // and her status stays on screen.
      retried.retrying();
      await _pumpEventQueue();
      expect(relaySession.subscriptions, [first]);
      expect(cache()['alice'], 'away');
      relaySession.emit(_presence('alice', 'online', 1));
      expect(cache()['alice'], 'online');

      // The next replacement resolves on the timeout, before its EOSE.
      notifier.track(['dave']);
      await _pumpEventQueue();
      final slow = relaySession.subscriptions.last;
      expect(slow.filter.authors, ['alice', 'bob', 'carol', 'dave']);
      slow.open();
      await _pumpEventQueue();
      expect(relaySession.subscriptions, [first, slow]);

      // Bob went offline after any earlier snapshot of him and before the new
      // stream covered him. Its late EOSE promotes it without blanking alice,
      // and the keys it newly covers are read again.
      relaySession.snapshot['bob'] = 'offline';
      relaySession.queried.clear();
      slow.ready();
      await _pumpEventQueue();
      expect(relaySession.subscriptions, [slow]);
      expect(
        relaySession.queried.expand((keys) => keys),
        containsAll(['bob', 'carol', 'dave']),
      );
      expect(
        relaySession.queried.expand((keys) => keys),
        isNot(contains('alice')),
      );
      expect(cache()['alice'], 'online');
      expect(cache()['bob'], 'offline');

      // A replacement that fails outright leaves the live stream and cache.
      notifier.track(['erin']);
      await _pumpEventQueue();
      relaySession.subscriptions.last.fail();
      await _pumpEventQueue();
      expect(relaySession.subscriptions, [slow]);
      expect(cache()['alice'], 'online');
      relaySession.emit(_presence('alice', 'away', 1));
      expect(cache()['alice'], 'away');
    },
  );

  test('WS event uses pubkey variable, not literal string', () async {
    // Regression test for the map key bug where `{...state, pubkey: status}`
    // used the literal string "pubkey" instead of the variable's value.
    final relaySession = _RecordingRelaySessionNotifier();
    final container = _buildContainer(relaySession: relaySession);
    addTearDown(container.dispose);

    container.read(presenceCacheProvider);
    await _pumpEventQueue();

    container.read(presenceCacheProvider.notifier).track([
      'deadbeef',
      'cafebabe',
    ]);
    await _pumpEventQueue();

    // Seed cafebabe -> offline, then set deadbeef online.
    relaySession.emit(_presence('cafebabe', 'offline'));
    relaySession.emit(_presence('deadbeef', 'online'));

    final cache = container.read(presenceCacheProvider);
    // deadbeef should be online (the actual pubkey, not a literal "pubkey" key).
    expect(cache['deadbeef'], 'online');
    // cafebabe should still be offline (not clobbered).
    expect(cache['cafebabe'], 'offline');
    // There should be no literal "pubkey" key in the map.
    expect(cache.containsKey('pubkey'), isFalse);
  });
}

NostrEvent _presence(String pubkey, String status, [int seq = 0]) => NostrEvent(
  id: 'evt-$pubkey-$status-$seq',
  pubkey: pubkey,
  createdAt: 1000,
  kind: EventKind.presenceUpdate,
  tags: const [],
  content: status,
  sig: 'sig',
);

Future<void> _pumpEventQueue() async {
  await Future<void>.delayed(Duration.zero);
  await Future<void>.delayed(Duration.zero);
}

ProviderContainer _buildContainer({
  required _RecordingRelaySessionNotifier relaySession,
}) {
  return ProviderContainer(
    overrides: [
      appLifecycleProvider.overrideWith(() => _FakeAppLifecycleNotifier()),
      relaySessionProvider.overrideWith(() => relaySession),
    ],
  );
}

class _RecordingRelaySessionNotifier extends RelaySessionNotifier {
  final List<NostrFilter> filters = [];
  final List<_FakeSubscription> subscriptions = [];
  final List<List<String>> queried = [];
  final Map<String, String> snapshot = {};

  /// When false, each subscription waits for the test to drive it.
  bool autoReady = true;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<void Function()> subscribeWithStatus(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
    required void Function(RelaySubscriptionStatus) onStatusChanged,
  }) {
    final subscription = _FakeSubscription(
      this,
      filter,
      onEvent,
      onClosed,
      onStatusChanged,
    );
    filters.add(filter);
    subscriptions.add(subscription);
    if (autoReady) {
      // Like an EOSE that arrives before the open future resolves.
      onStatusChanged(RelaySubscriptionStatus.ready);
      subscription.open();
    }
    return subscription.opened.future;
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final authors = filters.single.authors!;
    queried.add(authors);
    return [
      for (final key in authors)
        if (snapshot[key] case final status?) _presence(key, status),
    ];
  }

  /// Emit an event synchronously to all live subscribers.
  void emit(NostrEvent event) {
    for (final subscription in List.of(subscriptions)) {
      if (subscription.live) subscription.onEvent(event);
    }
  }
}

/// One subscription whose open, status and failure order the test controls.
class _FakeSubscription {
  _FakeSubscription(
    this.relay,
    this.filter,
    this.onEvent,
    this.onClosed,
    this.onStatusChanged,
  );

  final _RecordingRelaySessionNotifier relay;
  final NostrFilter filter;
  final void Function(NostrEvent) onEvent;
  final void Function(String message)? onClosed;
  final void Function(RelaySubscriptionStatus) onStatusChanged;
  final opened = Completer<void Function()>();
  bool live = true;

  void _remove() {
    live = false;
    relay.filters.remove(filter);
    relay.subscriptions.remove(this);
  }

  /// The open future resolves (EOSE, retryable CLOSED or the 500 ms timeout).
  void open() => opened.complete(_remove);
  void ready() => onStatusChanged(RelaySubscriptionStatus.ready);

  /// A retryable CLOSED: the open future resolves, then status is retrying.
  void retrying() {
    if (!opened.isCompleted) open();
    onStatusChanged(RelaySubscriptionStatus.retrying);
  }

  /// A terminal CLOSED on a subscription that is already open.
  void closeTerminal() {
    _remove();
    onClosed?.call('blocked: test');
  }

  /// A terminal CLOSED before the open future resolves.
  void fail() {
    _remove();
    onClosed?.call('blocked: test');
    opened.completeError(Exception('blocked: test'));
  }
}

class _FakeAppLifecycleNotifier extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}
