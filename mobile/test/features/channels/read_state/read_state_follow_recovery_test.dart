import 'dart:math';
import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/read_state/read_state_manager.dart';
import 'package:buzz/shared/read_state/thread_follows_format.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:shared_preferences_platform_interface/shared_preferences_platform_interface.dart';

import '../../../helpers/signed_event.dart';

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
  late _GatedStore store;
  var clock = 1800000000;

  setUp(() async {
    SharedPreferences.setMockInitialValues({});
    SharedPreferencesStorePlatform.instance = store = _GatedStore();
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

  String journalKey() => prefs.getKeys().singleWhere(
    (key) => key.startsWith('buzz.nip-rs.journal'),
  );

  Map<String, dynamic> journal() =>
      jsonDecode(prefs.getString(journalKey())!) as Map<String, dynamic>;

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

  // Holds each journal write of the flush in turn, including the one that
  // persists the frozen candidate's reservation before signing.
  for (var write = 1; write <= 6; write++) {
    test('C1: a tap during flush journal write $write is not acknowledged by '
        'an earlier payload', () async {
      final phone = await device();
      await phone.setThreadFollow(channel, 'a' * 64, true);
      final gate = Completer<void>();
      final held = Completer<void>();
      var seen = 0;
      store.holdJournal = (_) {
        if (++seen != write) return null;
        held.complete();
        return gate.future;
      };
      final sending = phone.flush();
      await Future.any([held.future, sending]);
      store.holdJournal = null;
      final unfollowing = phone.setThreadFollow(channel, 'a' * 64, false);
      gate.complete();
      await sending;
      expect(await unfollowing, isTrue);
      await phone.flush();
      await phone.flush();
      expect(phone.threadFollows[keyK]!.follow, isFalse);
      expect(relayHead().choices[keyK]!.follow, isFalse);

      phone.dispose(flushPending: false);
      await prefs.clear();
      final fresh = await device();
      expect(fresh.threadFollows[keyK]!.follow, isFalse);
    });
  }

  test('C2: a backup taken in the send delay after the first tap knows its '
      'own slot', () async {
    final phone = await device();
    await phone.setThreadFollow(channel, 'a' * 64, true);
    expect(journal()['followSlots'], hasLength(1));
    final early = backup();
    await phone.flush();
    clock -= 10;
    await phone.setThreadFollow(channel, 'a' * 64, false);
    await phone.flush();
    phone.dispose(flushPending: false);

    await restore(early);
    final restored = await device();
    await restored.flush();
    expect(restored.threadFollows[keyK], (follow: false, at: clock));
    expect(relay.follows(), hasLength(1));
    expect(relayHead().choices[keyK]!.follow, isFalse);
    restored.dispose(flushPending: false);
    await prefs.clear();
    final fresh = await device();
    expect(fresh.threadFollows[keyK]!.follow, isFalse);
  });

  for (final echo in _Echo.values) {
    test('I1: a real OK placeholder resolves the retained packet '
        '(${echo.name})', () async {
      relay.echo = echo;
      final phone = await device();
      await phone.setThreadFollow(channel, 'a' * 64, true);
      await phone.flush();
      await pumpEventQueue();
      expect(phone.syncError, isNull);
      expect(journal()['followPending'], isEmpty);
      expect(journal()['followEdits'], isEmpty);
      await phone.flush();
      expect(relay.attempts, hasLength(1));
      expect(relayHead().choices[keyK]!.follow, isTrue);
    });
  }

  test('I2: an accepted own head past the drift window floors the next '
      'reservation', () async {
    final phone = await device();
    await phone.setThreadFollow(channel, 'a' * 64, true);
    await phone.flush();
    final own = relay.follows().single;
    final ahead = signedBy(
      keys,
      NostrEvent(
        id: '1' * 64,
        pubkey: keys.public,
        createdAt: clock + 1000,
        kind: EventKind.readState,
        tags: own.tags,
        content: crypto.encrypt(
          ThreadFollowsBlob(
            clientId: relayHead().clientId,
            entries: {
              keyK: {'follow': true, 'at': clock},
            },
          ).encode(),
        ),
        sig: 'sig',
      ),
    );
    relay.deliver(ahead);
    await phone.setThreadFollow(channel, 'a' * 64, false);
    expect(
      (journal()['followEdits'] as Map)[keyK],
      greaterThan(ahead.createdAt),
    );
    // The relay fake keeps the newer event, so the unfollow must outrank it.
    relay.retained[own.tags.first[1]] = ahead;
    await phone.flush();
    expect(relay.follows().single.createdAt, greaterThan(ahead.createdAt));
    expect(relayHead().choices[keyK]!.follow, isFalse);
  });

  test('I3: an unreadable own head holds a retained retry and refuses taps; '
      'frontiers still publish', () async {
    final phone = await device();
    await phone.setThreadFollow(channel, 'a' * 64, true);
    relay.dropNext = true;
    await phone.flush();
    final pending = Map.of(journal()['followPending'] as Map);
    expect(pending, hasLength(1));
    final coordinate = pending.keys.single as String;
    relay.retained[coordinate] = NostrEvent(
      id: '2' * 64,
      pubkey: keys.public,
      createdAt: clock + 5,
      kind: EventKind.readState,
      tags: [
        ['d', coordinate],
        ['t', threadFollowsTag],
      ],
      content: crypto.encrypt('{"v":2}'),
      sig: 'sig',
    );
    await phone.reinitializeRemote();
    relay.attempts.clear();
    phone.markContextRead(channel, clock - 100);
    await phone.flush();
    expect(relay.attempts, isEmpty);
    expect(phone.syncError, isNotNull);
    expect(journal()['followPending'], pending);
    expect(journal()['followEdits'], contains(keyK));
    expect(await phone.setThreadFollow(channel, 'b' * 64, true), isFalse);
    expect(
      relay.retained.keys.where((d) => d.startsWith('read-state:')),
      isNotEmpty,
    );
  });

  test('startup gate: a tap before the first recovery is refused and '
      'changes nothing; after recovery it is accepted', () async {
    final first = await device();
    expect(await first.setThreadFollow(channel, 'a' * 64, true), isTrue);
    first.dispose(flushPending: false);
    final saved = Map.of(journal());
    relay.offline = true;
    // A cold start from a journal a recovered writer saved.
    final phone = await device();
    expect(phone.threadFollowsReady, isFalse);
    expect(phone.threadFollows[keyK]!.follow, isTrue);
    expect(await phone.setThreadFollow(channel, 'a' * 64, false), isFalse);
    expect(await phone.setThreadFollow(channel, 'b' * 64, true), isFalse);
    expect(phone.threadFollows[keyK]!.follow, isTrue);
    expect(phone.threadFollows.containsKey(keyB), isFalse);
    expect(journal()['followEdits'], saved['followEdits']);
    expect(journal()['ownFollows'], saved['ownFollows']);
    expect('${phone.syncError}', contains(threadFollowsNotReadyMessage));
    relay.offline = false;
    await phone.reinitializeRemote();
    expect(phone.threadFollowsReady, isTrue);
    expect(await phone.setThreadFollow(channel, 'b' * 64, true), isTrue);
    expect(phone.threadFollows[keyB]!.follow, isTrue);
  });

  test('startup gate: stays shut until the recovery save succeeds', () async {
    relay.offline = true;
    final phone = await device();
    relay.offline = false;
    final gate = Completer<void>();
    store.holdJournal = (_) => gate.future;
    final recovery = phone.reinitializeRemote();
    await pumpEventQueue();
    // The snapshot is merged, but its save is held.
    expect(await phone.setThreadFollow(channel, 'a' * 64, true), isFalse);
    store.holdJournal = null;
    store.failJournal = true;
    gate.complete();
    await recovery;
    expect(await phone.setThreadFollow(channel, 'a' * 64, true), isFalse);
    expect(phone.threadFollows.containsKey(keyK), isFalse);
    store.failJournal = false;
    // The next reconnect's ready reads the snapshot again.
    relay.ready();
    await pumpEventQueue();
    expect(await phone.setThreadFollow(channel, 'a' * 64, true), isTrue);
  });

  test('startup gate: reads and channel mark unread are not gated', () async {
    relay.offline = true;
    final offline = await device();
    offline.markContextRead(channel, clock - 100);
    expect(offline.getEffectiveTimestamp(channel), clock - 100);
    offline.dispose(flushPending: false);
    relay.offline = false;
    final writer = await device();
    await writer.setThreadFollow(channel, 'a' * 64, true);
    await writer.flush();
    writer.dispose(flushPending: false);
    // Recovered, but an unreadable own head keeps follows not ready.
    final coordinate = relay.follows().single.tags.firstWhere(
      (tag) => tag.first == 'd',
    )[1];
    relay.retained[coordinate] = NostrEvent(
      id: '2' * 64,
      pubkey: keys.public,
      createdAt: clock + 5,
      kind: EventKind.readState,
      tags: [
        ['d', coordinate],
        ['t', threadFollowsTag],
      ],
      content: crypto.encrypt('{"v":2}'),
      sig: 'sig',
    );
    final phone = await device();
    expect(phone.threadFollowsReady, isFalse);
    expect(await phone.setThreadFollow(channel, 'b' * 64, true), isFalse);
    phone.markContextRead('other', clock - 50);
    expect(phone.getEffectiveTimestamp('other'), clock - 50);
    expect(phone.markChannelUnread(channel), isTrue);
  });

  test('I4: an import is not settled by a later tentative entry', () async {
    final phone = await device();
    final gates = [for (var i = 0; i < 3; i++) Completer<void>()];
    var writes = 0;
    store.holdJournal = (_) => writes < 3 ? gates[writes++].future : null;
    // A imports K; its save is held.
    final a = phone.importLegacyFollows({keyK: true});
    await pumpEventQueue();
    // B sees A's tentative K and awaits a save of its own.
    final b = phone.importLegacyFollows({keyK: true});
    await pumpEventQueue();
    // A's save fails and A reverts K.
    store.failJournal = true;
    gates[0].complete();
    expect(await a, isEmpty);
    expect(phone.threadFollows.containsKey(keyK), isFalse);
    // C imports K again while B's save is still held.
    final c = phone.importLegacyFollows({keyK: true});
    await pumpEventQueue();
    // B's save succeeds; C's then fails.
    store.failJournal = false;
    gates[1].complete();
    expect(await b, isEmpty);
    store.failJournal = true;
    gates[2].complete();
    expect(await c, isEmpty);
    store.failJournal = false;
    // A later save drops the reverted entry from the journal.
    phone.markContextRead(channel, clock - 100);
    await phone.flush();
    phone.dispose(flushPending: false);
    // Nothing was settled, so the legacy source still holds K to import.
    final restarted = await device();
    expect(restarted.threadFollows.containsKey(keyK), isFalse);
    expect(await restarted.importLegacyFollows({keyK: true}), {keyK});
  });

  test('I4: a legacy import is not settled until it is journaled', () async {
    final phone = await device();
    store.failJournal = true;
    expect(await phone.importLegacyFollows({keyK: true}), isEmpty);
    expect(phone.threadFollows.containsKey(keyK), isFalse);
    store.failJournal = false;
    expect(await phone.importLegacyFollows({keyK: true}), {keyK});
    expect(phone.threadFollows[keyK], (follow: true, at: 0));
  });

  test('I4: an overlapping import is not settled by an unsaved one', () async {
    final phone = await device();
    final gate = Completer<void>();
    store.holdJournal = (_) => gate.future;
    final first = phone.importLegacyFollows({keyK: true});
    final second = phone.importLegacyFollows({keyK: true});
    await pumpEventQueue();
    // The first import's save, still held, then fails.
    store.holdJournal = null;
    store.failJournal = true;
    gate.complete();
    expect(await first, isEmpty);
    expect(await second, isEmpty);
    expect(phone.threadFollows.containsKey(keyK), isFalse);
    store.failJournal = false;
    expect(await phone.importLegacyFollows({keyK: true}), {keyK});
  });

  test('I4: an exhausted reservation leaves no import residue', () async {
    final phone = await device();
    clock = 4294967295;
    expect(await phone.setThreadFollow(channel, 'b' * 64, true), isTrue);
    expect(await phone.importLegacyFollows({keyK: true}), isEmpty);
    expect(phone.threadFollows.containsKey(keyK), isFalse);
    expect(await phone.importLegacyFollows({keyK: true}), isEmpty);
    expect(phone.threadFollows.containsKey(keyK), isFalse);
  });

  test('I5: capacity pruning keeps a settled explicit unfollow', () async {
    final first = await device();
    first.dispose(flushPending: false);
    // A journal at the slot ceiling: settled own follows a peer outranks,
    // and one settled own unfollow the peer also outranks.
    final keyU = threadFollowKey(channel, 'c' * 64);
    final many = [
      for (var i = 0; i < 7000; i++)
        threadFollowKey(channel, i.toRadixString(16).padLeft(64, '0')),
    ];
    final saved = journal();
    saved['ownFollows'] = {
      for (final key in many) key: {'follow': true, 'at': 1},
      keyU: {'follow': false, 'at': 1},
    };
    // Peer slots of 300 keys each, as real coordinates must stay small.
    final outranked = [...many, keyU];
    final peers = {
      for (var i = 0; i * 300 < outranked.length; i++)
        i.toRadixString(16).padLeft(32, 'f'): outranked.sublist(
          i * 300,
          min((i + 1) * 300, outranked.length),
        ),
    };
    String peerId(String slot) => '3${slot.padLeft(63, '0')}';
    saved['followHeads'] = {
      for (final MapEntry(key: slot, value: roots) in peers.entries)
        slot: {
          'createdAt': 10,
          'id': peerId(slot),
          'threads': {
            for (final key in roots) key: {'follow': true, 'at': 2},
          },
        },
    };
    await prefs.setString(journalKey(), jsonEncode(saved));
    // The peer heads are still on the relay: a complete snapshot without
    // them would mean their owner deleted them.
    for (final MapEntry(key: slot, value: roots) in peers.entries) {
      relay.retained['$threadFollowsDTagPrefix$slot'] = NostrEvent(
        id: peerId(slot),
        pubkey: keys.public,
        createdAt: 10,
        kind: EventKind.readState,
        tags: [
          ['d', '$threadFollowsDTagPrefix$slot'],
          ['t', threadFollowsTag],
        ],
        content: crypto.encrypt(
          ThreadFollowsBlob(
            clientId: 'peer',
            entries: {
              for (final key in roots) key: {'follow': true, 'at': 2},
            },
          ).encode(),
        ),
        sig: 'sig',
      );
    }
    final phone = await device();
    expect(await phone.setThreadFollow(channel, 'd' * 64, true), isTrue);
    await phone.flush();
    final own = {
      for (final event in relay.follows())
        ...decodeThreadFollowsBlob(crypto.decrypt(event.content))!.choices,
    };
    expect(own[keyU], (follow: false, at: 1));
  });

  test('own heads at the same second: the lower event ID wins in either '
      'arrival order', () async {
    final phone = await device();
    await phone.setThreadFollow(channel, 'a' * 64, true);
    await phone.flush();
    final own = relay.follows().single;
    final clientId = relayHead().clientId;
    NostrEvent head(bool follow) => signedBy(
      keys,
      NostrEvent(
        id: '',
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
        sig: '',
      ),
    );
    // Real IDs are hashes, so either choice can hold the lower one.
    final [low, high] = [head(false), head(true)]
      ..sort((a, b) => a.id.compareTo(b.id));
    final wins = decodeThreadFollowsBlob(
      crypto.decrypt(low.content),
    )!.choices[keyK]!.follow;

    relay.deliver(high);
    relay.deliver(low);
    expect(phone.threadFollows[keyK]!.follow, wins);
    relay.deliver(high);
    expect(phone.threadFollows[keyK]!.follow, wins);

    phone.dispose(flushPending: false);
    await prefs.clear();
    final other = await device();
    relay.deliver(low);
    relay.deliver(high);
    expect(other.threadFollows[keyK]!.follow, wins);
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
  bool offline = false;
  _Echo echo = _Echo.beforeOk;
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
  }) async {
    if (offline) throw TimeoutException('relay unreachable');
    return retained.values.toList();
  }

  final List<void Function(RelaySubscriptionStatus)> _statusListeners = [];

  /// Reports the live subscription ready again, as a reconnect's replay does.
  void ready() {
    for (final listener in List.of(_statusListeners)) {
      listener(RelaySubscriptionStatus.ready);
    }
  }

  @override
  Future<void Function()> subscribeWithStatus(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
    required void Function(RelaySubscriptionStatus status) onStatusChanged,
    bool resumeSince = true,
  }) async {
    final unsubscribe = await subscribe(filter, onEvent, onClosed: onClosed);
    _statusListeners.add(onStatusChanged);
    return () {
      _statusListeners.remove(onStatusChanged);
      unsubscribe();
    };
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    if (offline) throw TimeoutException('relay unreachable');
    _live.add(onEvent);
    return () => _live.remove(onEvent);
  }

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
    Future<bool> Function()? mayPublish,
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
    switch (echo) {
      case _Echo.beforeOk:
        deliver(event);
      case _Echo.afterOk:
        Timer.run(() => deliver(event));
      case _Echo.none:
        break;
    }
    // What `RelaySessionNotifier._handleOk` returns: the ID only.
    return NostrEvent(
      id: event.id,
      pubkey: '',
      createdAt: 0,
      kind: 0,
      tags: [],
      content: '',
      sig: '',
    );
  }
}

enum _Echo { beforeOk, afterOk, none }

/// Can hold a matching journal write, or fail every journal write.
class _GatedStore extends InMemorySharedPreferencesStore {
  _GatedStore() : super.empty();
  Future<void>? Function(String encoded)? holdJournal;
  bool failJournal = false;

  @override
  Future<bool> setValue(String valueType, String key, Object value) async {
    if (key.contains('buzz.nip-rs.journal')) {
      if (holdJournal?.call(value as String) case final gate?) await gate;
      if (failJournal) return false;
    }
    return super.setValue(valueType, key, value);
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
    Future<bool> Function()? mayPublish,
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
      mayPublish: mayPublish,
    );
  }
}
