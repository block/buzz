import 'dart:async';

import 'package:buzz/shared/read_state/read_state_manager.dart';
import 'package:buzz/shared/read_state/thread_follows_format.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

/// The shared follow-recovery witnesses
/// (`PLANS/READ_STATE_FOLLOW_SYNC_RULES_2026_11_14.md` §8), against the real
/// manager, journal and signing path: the accepted own head, pending edits
/// and the exact in-flight packet stay separate.
void main() {
  const channel = 'chan-1';
  final keyK = threadFollowKey(channel, 'a' * 64);
  final keyB = threadFollowKey(channel, 'b' * 64);
  late nostr.Keys keys;
  late ReadStateCrypto crypto;
  late _HoldingRelay relay;
  late SharedPreferences prefs;
  var clock = 1800000000;

  setUp(() async {
    SharedPreferences.setMockInitialValues({});
    prefs = await SharedPreferences.getInstance();
    keys = nostr.Keys.generate();
    crypto = ReadStateCrypto.tryCreate(nsec: keys.nsec, pubkey: keys.public)!;
    relay = _HoldingRelay();
    clock = 1800000000;
  });

  Future<ReadStateManager> device() async {
    final manager = ReadStateManager(
      pubkey: keys.public,
      relayUrl: 'https://phone.example',
      prefs: prefs,
      crypto: crypto,
      relaySession: relay,
      signedEventRelay: SignedEventRelay(session: relay, nsec: keys.nsec),
      remoteEnabled: true,
      onChanged: () {},
      now: () => clock,
    );
    addTearDown(() => manager.dispose(flushPending: false));
    await manager.initialize();
    return manager;
  }

  Map<String, Object> backup() => {
    for (final key in prefs.getKeys()) key: prefs.get(key)!,
  };

  Future<void> restore(Map<String, Object> saved) async {
    await prefs.clear();
    for (final MapEntry(:key, :value) in saved.entries) {
      switch (value) {
        case String():
          await prefs.setString(key, value);
        case int():
          await prefs.setInt(key, value);
        case bool():
          await prefs.setBool(key, value);
        case List<String>():
          await prefs.setStringList(key, value);
      }
    }
  }

  ThreadFollowsBlob relayHead() =>
      decodeThreadFollowsBlob(crypto.decrypt(relay.follows().single.content))!;

  test('W1: an unfollow at a rolled-back clock survives restoring an older '
      'journal', () async {
    final phone = await device();
    phone.setThreadFollow(channel, 'a' * 64, true);
    await phone.flush();
    final clean = backup();

    clock -= 3600;
    phone.setThreadFollow(channel, 'a' * 64, false);
    await phone.flush();
    phone.dispose(flushPending: false);

    await restore(clean);
    final restored = await device();
    await restored.flush();
    expect(restored.threadFollows[keyK], (follow: false, at: clock));
    expect(relayHead().choices[keyK], (follow: false, at: clock));

    // A fresh reader agrees.
    await prefs.clear();
    final fresh = await device();
    expect(fresh.threadFollows[keyK], (follow: false, at: clock));
  });

  test('W2: a same-second unfollow survives the earlier follow\'s late echo '
      'and OK', () async {
    final phone = await device();
    final gate = relay.hold = Completer();
    phone.setThreadFollow(channel, 'a' * 64, true);
    final sending = phone.flush();
    await relay.held.future;
    final p = relay.inFlight!;

    // Same action second, while P is in flight.
    phone.setThreadFollow(channel, 'a' * 64, false);
    relay.deliver(p); // Echo before the OK.
    expect(phone.threadFollows[keyK]!.follow, isFalse);
    gate.complete();
    await sending;
    expect(phone.threadFollows[keyK]!.follow, isFalse);

    await phone.flush(); // The successor.
    final q = relay.follows().single;
    expect(q.createdAt, greaterThan(p.createdAt));
    relay.deliver(p); // A stale echo after the successor was accepted.
    expect(phone.threadFollows[keyK], (follow: false, at: clock));
    expect(relayHead().choices[keyK], (follow: false, at: clock));
    await phone.flush();
    expect(relay.follows().single.id, q.id);

    phone.dispose(flushPending: false);
    await prefs.clear();
    final fresh = await device();
    expect(fresh.threadFollows[keyK]!.follow, isFalse);
  });

  test('no omission: a successor carries an edit captured by a packet whose '
      'outcome is unknown', () async {
    final phone = await device();
    final gate = relay.hold = Completer();
    phone.setThreadFollow(channel, 'a' * 64, true);
    final sending = phone.flush();
    await relay.held.future;
    final p = relay.inFlight!;
    phone.setThreadFollow(channel, 'b' * 64, true);
    relay.dropNext = true; // P times out unseen by the relay.
    gate.complete();
    await sending;
    expect(phone.syncError, isNotNull);

    // P is replayed exactly, then its successor carries both edits.
    await phone.flush();
    expect(relayHead().choices.keys, unorderedEquals([keyK, keyB]));
    expect(relay.follows().single.createdAt, greaterThan(p.createdAt));
    relay.deliver(p); // A late duplicate of P loses.
    expect(phone.threadFollows.keys, unorderedEquals([keyK, keyB]));
    await phone.flush();
    expect(relayHead().choices.keys, unorderedEquals([keyK, keyB]));
  });

  test('own heads at the same second: the lower event ID wins in either '
      'arrival order', () async {
    final phone = await device();
    phone.setThreadFollow(channel, 'a' * 64, true);
    await phone.flush();
    final own = relay.follows().single;
    final clientId = relayHead().clientId;
    NostrEvent head(String id, bool follow) => NostrEvent(
      id: id,
      pubkey: keys.public,
      createdAt: own.createdAt + 10,
      kind: EventKind.readState,
      tags: own.tags,
      content: crypto.encrypt(
        ThreadFollowsBlob(
          clientId: clientId,
          entries: {
            keyK: {'follow': follow, 'at': clock},
          },
        ).encode(),
      ),
      sig: 'sig',
    );
    final low = head('1' * 64, false);
    final high = head('e' * 64, true);

    relay.deliver(high);
    relay.deliver(low);
    expect(phone.threadFollows[keyK]!.follow, isFalse);
    relay.deliver(high);
    expect(phone.threadFollows[keyK]!.follow, isFalse);

    phone.dispose(flushPending: false);
    await prefs.clear();
    final other = await device();
    relay.deliver(low);
    relay.deliver(high);
    expect(other.threadFollows[keyK]!.follow, isFalse);
  });
}

/// The newest event per coordinate, served as the snapshot. A publish can be
/// held open or dropped as unseen.
class _HoldingRelay extends RelaySessionNotifier {
  final retained = <String, NostrEvent>{};
  final _live = <void Function(NostrEvent)>[];
  Completer<void>? hold;
  Completer<void> held = Completer();
  NostrEvent? inFlight;
  bool dropNext = false;

  void deliver(NostrEvent event) {
    for (final onEvent in List.of(_live)) {
      onEvent(event);
    }
  }

  List<NostrEvent> follows() => [
    for (final event in retained.values)
      if (threadFollowsSlotOf(event) != null) event,
  ];

  @override
  Future<List<NostrEvent>> fetchReadStateSnapshot({
    Duration timeout = const Duration(seconds: 8),
  }) async => retained.values.toList();

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    _live.add(onEvent);
    return () => _live.remove(onEvent);
  }

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (hold case final gate? when threadFollowsSlotOf(event) != null) {
      hold = null;
      inFlight = event;
      held.complete();
      held = Completer();
      await gate.future;
    }
    if (dropNext) {
      dropNext = false;
      throw TimeoutException('relay timeout');
    }
    final d = event.tags.firstWhere((tag) => tag.first == 'd')[1];
    final head = retained[d];
    if (head == null ||
        event.createdAt > head.createdAt ||
        event.createdAt == head.createdAt && event.id.compareTo(head.id) < 0) {
      retained[d] = event;
    }
    deliver(event);
    return event;
  }
}
