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

  Future<ReadStateManager> device({SignedEventRelay? signer}) async {
    final manager = ReadStateManager(
      pubkey: keys.public,
      relayUrl: 'https://phone.example',
      prefs: prefs,
      crypto: crypto,
      relaySession: relay,
      signedEventRelay:
          signer ?? SignedEventRelay(session: relay, nsec: keys.nsec),
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
    await phone.setThreadFollow(channel, 'a' * 64, true);
    await phone.flush();
    final clean = backup();

    clock -= 3600;
    await phone.setThreadFollow(channel, 'a' * 64, false);
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
    await phone.setThreadFollow(channel, 'a' * 64, true);
    final sending = phone.flush();
    await relay.held.future;
    final p = relay.inFlight!;

    // Same action second, while P is in flight.
    await phone.setThreadFollow(channel, 'a' * 64, false);
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
    await phone.setThreadFollow(channel, 'a' * 64, true);
    final sending = phone.flush();
    await relay.held.future;
    final p = relay.inFlight!;
    await phone.setThreadFollow(channel, 'b' * 64, true);
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

  test('a tap whose journal write fails is refused and reverted', () async {
    final phone = await device();
    expect(await phone.setThreadFollow(channel, 'a' * 64, true), isTrue);
    // Another writer claims the journal, so this one's save fails.
    await device();
    expect(await phone.setThreadFollow(channel, 'a' * 64, false), isFalse);
    expect(phone.threadFollows[keyK]!.follow, isTrue);
    expect(phone.syncError, isNotNull);
  });

  test('a reservation past uint32 is refused, not wrapped', () async {
    final phone = await device();
    clock = 4294967295;
    expect(await phone.setThreadFollow(channel, 'a' * 64, true), isTrue);
    expect(await phone.setThreadFollow(channel, 'b' * 64, true), isFalse);
    expect(phone.threadFollows.containsKey(keyB), isFalse);
    expect(phone.syncError, isA<StateError>());
  });

  test('no omission through a slot spill: every captured edit reaches a '
      'slot', () async {
    final phone = await device();
    final first = [for (var i = 0; i < 200; i++) i.toRadixString(16)];
    for (final id in first) {
      await phone.setThreadFollow(channel, id.padLeft(64, '0'), true);
    }
    final gate = relay.hold = Completer();
    final sending = phone.flush();
    await relay.held.future;
    // While the first packet is in flight, enough taps to spill a slot.
    final more = [for (var i = 200; i < 500; i++) i.toRadixString(16)];
    for (final id in more) {
      await phone.setThreadFollow(channel, id.padLeft(64, '0'), false);
    }
    gate.complete();
    await sending;
    await phone.flush();
    expect(phone.syncError, isNull);
    final follows = relay.follows();
    expect(follows.length, greaterThan(1));
    final keys = {
      for (final event in follows)
        ...decodeThreadFollowsBlob(crypto.decrypt(event.content))!.choices.keys,
    };
    expect(keys, {
      for (final id in [...first, ...more])
        threadFollowKey(channel, id.padLeft(64, '0')),
    });
  });

  test(
    'an unknown send outcome is retried with the exact signed bytes',
    () async {
      final phone = await device();
      await phone.setThreadFollow(channel, 'a' * 64, true);
      relay.dropNext = true;
      await phone.flush();
      expect(phone.syncError, isNotNull);
      final attempt = relay.attempts.single;
      clock += 30;
      await phone.setThreadFollow(channel, 'b' * 64, true);
      await phone.flush();
      expect(relay.attempts[1].toJson(), attempt.toJson());
      expect(relay.attempts.length, 3); // Then the successor with both.
      expect(relayHead().choices.keys, unorderedEquals([keyK, keyB]));
    },
  );

  test(
    'a writer retired while signing neither sends nor loses the edit',
    () async {
      final signGate = Completer<void>();
      final signing = Completer<void>();
      final phone = await device(
        signer: _GatedSigner(relay, keys.nsec, signGate, signing),
      );
      await phone.setThreadFollow(channel, 'a' * 64, true);
      final sending = phone.flush();
      await signing.future;
      final replacement = await device(); // Retires [phone].
      signGate.complete();
      await sending;
      expect(relay.follows(), isEmpty);
      await replacement.flush();
      expect(relayHead().choices[keyK]!.follow, isTrue);
    },
  );

  test('own heads at the same second: the lower event ID wins in either '
      'arrival order', () async {
    final phone = await device();
    await phone.setThreadFollow(channel, 'a' * 64, true);
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
  final attempts = <NostrEvent>[];

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
    if (threadFollowsSlotOf(event) != null) attempts.add(event);
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

/// Signs only after [gate] opens, completing [signing] once a follow packet
/// reaches it.
class _GatedSigner extends SignedEventRelay {
  final Completer<void> gate;
  final Completer<void> signing;
  _GatedSigner(
    RelaySessionNotifier session,
    String nsec,
    this.gate,
    this.signing,
  ) : super(session: session, nsec: nsec);

  @override
  Future<NostrEvent> submit({
    required int kind,
    required String content,
    required List<List<String>> tags,
    int? createdAt,
    FutureOr<void> Function(NostrEvent event)? onSigned,
  }) async {
    if (tags.any((tag) => tag.first == 't' && tag[1] == threadFollowsTag)) {
      if (!signing.isCompleted) signing.complete();
      await gate.future;
    }
    return super.submit(
      kind: kind,
      content: content,
      tags: tags,
      createdAt: createdAt,
      onSigned: onSigned,
    );
  }
}
