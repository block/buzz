import 'dart:convert';
import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/shared/read_state/read_state_format.dart';
import 'package:buzz/shared/read_state/read_state_packing.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/shared/read_state/read_state_manager.dart';
import 'package:buzz/shared/read_state/read_state_storage.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:shared_preferences_platform_interface/shared_preferences_platform_interface.dart';
import 'package:buzz/shared/relay/read_state_snapshot.dart';

void main() {
  final corpus =
      jsonDecode(
            File('../test-fixtures/read-state-vectors.json').readAsStringSync(),
          )
          as Map<String, dynamic>;
  Map<String, int> map(dynamic value) => Map<String, int>.from(value as Map);
  for (final vector in corpus['packing'] as List) {
    test('shared packing: ${vector['id']}', () {
      final generated = vector['generate'] as Map?;
      final frontiers = generated == null
          ? map(vector['frontiers'])
          : {
              for (var i = 0; i < generated['count']; i++)
                '${generated['keyPrefix']}${i.toRadixString(16).padLeft(64, '0')}':
                    (generated['valueStart'] as int) +
                    i * (generated['valueStep'] as int),
            };
      final blobs = packReadStateContexts(
        frontiers,
        clientId: vector['clientId'],
        maxBytes: vector['maxPlaintextBytes'],
      );
      expect(mergeReadStateContexts(blobs), frontiers);
      expect([
        for (final blob in blobs)
          utf8
              .encode(
                jsonEncode(
                  ReadStateBlob(
                    clientId: vector['clientId'],
                    contexts: blob,
                  ).toJson(),
                ),
              )
              .length,
      ], vector['expectedPlaintextBytes']);
      if (generated != null) {
        final ranges = vector['expectedBlobRanges'] as List;
        expect(blobs.length, ranges.length);
        for (var i = 0; i < ranges.length; i++) {
          String key(int n) =>
              '${generated['keyPrefix']}${n.toRadixString(16).padLeft(64, '0')}';
          expect(blobs[i].length, ranges[i]['count']);
          expect(blobs[i].keys.first, key(ranges[i]['firstIndex']));
          expect(blobs[i].keys.last, key(ranges[i]['lastIndex']));
        }
      }
      if (vector['expectedBlobKeys'] != null) {
        expect(
          blobs.map((blob) => blob.keys.toList()).toList(),
          vector['expectedBlobKeys'],
        );
      }
    });
  }
  for (final vector in corpus['evaluation'] as List) {
    test('shared evaluation: ${vector['id']}', () {
      final merged = mergeReadStateContexts(
        (vector['states'] as List).map(map),
      );
      final state = ReadStateState(
        isReady: true,
        pubkey: 'fixture',
        contexts: merged,
        version: 0,
        retentionFloor: readStateFloor(corpus['now']),
      );
      final effective = maxReadAt([
        state.effectiveTimestamp(vector['key']),
        for (final parent in vector['parents'])
          state.effectiveTimestamp(parent),
      ])!;
      expect(effective, vector['expectedEffective']);
      expect(
        (vector['messageAt'] as int) > effective,
        vector['expectedUnread'],
      );
    });
  }
  for (final vector in corpus['placement'] as List) {
    test('shared placement: ${vector['id']}', () {
      final old = [for (final slot in vector['slots']) map(slot['frontiers'])];
      final next = packReadStateContexts(
        {...mergeReadStateContexts(old), ...map(vector['advance'])},
        clientId: vector['clientId'],
        previous: old,
        maxBytes: vector['maxPlaintextBytes'],
      );
      for (var index = 0; index < old.length; index++) {
        for (final key in old[index].keys) {
          expect(next[index][key], old[index][key]);
        }
      }
    });
  }
  for (final vector in corpus['ownership'] as List) {
    test('shared owner: ${vector['id']}', () async {
      final harness = await _Owner.create(corpus['now']);
      harness.advance(map(vector['local']));
      for (final remote in vector['remote']) {
        harness.remote(map(remote));
      }
      await harness.manager.initialize();
      if (vector['localAdvance'] != null) {
        harness.advance(map(vector['localAdvance']));
      }
      await harness.manager.flush();
      expect(harness.manager.syncError, isNull);
      expect(harness.manager.effectiveContexts, map(vector['expectedMerged']));
      expect(harness.published, map(vector['expectedPublished']));
    });
  }
  for (final vector in corpus['writePolicy'] as List) {
    test('shared owner write policy: ${vector['id']}', () async {
      final harness = await _Owner.create(corpus['now']);
      // The production owner has no mention opt-in: all individual-message
      // actions take this same non-publishing path, including direct mentions.
      expect(vector['messageIsDirectMention'], isTrue);
      harness.advance(map(vector['localObserved']));
      await harness.manager.flush();
      expect(harness.manager.syncError, isNull);
      expect(harness.published, map(vector['expectedPublished']));
      for (final key in vector['expectedAbsentKeys']) {
        expect(harness.published, isNot(contains(key)));
      }
      harness.restart();
      expect(harness.manager.effectiveContexts, map(vector['localObserved']));
      await harness.manager.flush();
      expect(harness.published, map(vector['expectedPublished']));
    });
  }
  for (final vector in corpus['pruning'] as List) {
    test('shared owner pruning: ${vector['id']}', () async {
      final harness = await _Owner.create(corpus['now']);
      harness.advance(map(vector['frontiers']));
      await harness.manager.flush();
      expect(harness.manager.syncError, isNull);
      for (final key in vector['expectedRequiredKeys']) {
        expect(harness.published, containsPair(key, vector['frontiers'][key]));
      }
      for (final key in vector['expectedAbsentKeys']) {
        expect(harness.published, isNot(contains(key)));
      }
      expect(
        harness.published.keys.toSet().difference({
          ...vector['expectedRequiredKeys'],
          ...vector['expectedOptionalInertKeys'],
        }),
        isEmpty,
      );
    });
  }
  for (final vector in corpus['evaluation'] as List) {
    if (vector['carrierCreatedAt'] == null) continue;
    test('shared owner old carrier: ${vector['id']}', () async {
      final harness = await _Owner.create(corpus['now']);
      for (final remote in vector['states']) {
        harness.remote(map(remote), createdAt: vector['carrierCreatedAt']);
      }
      await harness.manager.initialize();
      expect(
        harness.manager.getEffectiveTimestamp(vector['key']),
        vector['expectedEffective'],
      );
      await harness.manager.flush();
      expect(
        harness.published,
        isEmpty,
      ); // Reading a peer is not local ownership.
    });
  }

  for (final rollback in [0, 60]) {
    test(
      'pending absent from snapshot gets newer successor rollback=$rollback',
      () async {
        final harness = await _Owner.create(corpus['now']);
        harness.session.failAt = 1;
        harness.session.dropBeforeAccept = true;
        harness.advance({'first': harness.now});
        await harness.manager.flush();
        expect(harness.manager.syncError, isNotNull);
        final pending = harness.session.attempts.single;
        expect(harness.session.retained, isEmpty);
        harness.now -= rollback;
        harness.restart();
        harness.advance({'second': harness.now});
        harness.session.failAt = null;
        await harness.manager.flush();
        expect(harness.manager.syncError, isNull);
        expect(harness.session.attempts[1].toJson(), pending.toJson());
        expect(
          harness.session.attempts[2].createdAt,
          greaterThan(pending.createdAt),
        );
        expect(harness.published, {
          'first': pending.createdAt,
          'second': harness.now,
        });
        harness.restart();
        await harness.manager.initialize();
        expect(harness.manager.getEffectiveTimestamp('second'), harness.now);
      },
    );
  }

  test('flush alone retries a failed durable journal before sending', () async {
    final store = _FailingJournalStore();
    final harness = await _Owner.create(corpus['now'], store: store);
    harness.advance({'room': harness.now});
    await harness.manager.flush();
    expect(harness.manager.syncError, isNotNull);
    expect(harness.session.attempts, isEmpty);
    store.fail = false;
    await harness.manager.flush();
    expect(harness.manager.syncError, isNull);
    expect(harness.published, {'room': harness.now});
    expect(store.writes, greaterThan(1));
  });

  test(
    'reconnect during pending save requires a fresh capacity snapshot',
    () async {
      final store = _FailingJournalStore()..fail = false;
      final harness = await _Owner.create(corpus['now'], store: store);
      var interrupted = false;
      store.afterSave = (value) {
        final journal = jsonDecode(value as String) as Map;
        if (!interrupted && (journal['pending'] as Map).isNotEmpty) {
          interrupted = true;
          harness.session.debugSupersedeConnection();
        }
      };
      harness.advance({'room': harness.now});
      await harness.manager.flush();
      expect(interrupted, isTrue);
      expect(harness.session.attempts, isEmpty);
      expect(harness.manager.syncError, isNotNull);
      final journalKey =
          '${harness.keys.public}:${Uri.encodeComponent('https://relay.example')}';
      final saved = ReadStateStorage(harness.prefs).readJournal(journalKey)!;
      final pending = (saved['pending'] as Map).values.single;
      // The new connection discovers capacity consumed during the gap. Neither
      // the exact pending retry nor its successor may bypass this new snapshot.
      harness.session.history.addAll(
        List.generate(4096, (i) => harness.unrelated('other-$i', '')),
      );
      await harness.manager.flush();
      expect(harness.session.attempts, isEmpty);
      expect(harness.manager.syncError, isNotNull);
      harness.session.history.clear();
      await harness.manager.flush();
      expect(harness.manager.syncError, isNull);
      expect(harness.session.attempts.single.toJson(), pending);
      expect(harness.published, {'room': harness.now});
    },
  );

  test('retired ACK cannot replace successor journal', () async {
    final harness = await _Owner.create(corpus['now']);
    final gate = Completer<void>();
    final started = Completer<void>();
    harness.session.ackGate = gate;
    harness.session.beforePublish = (_) {
      if (!started.isCompleted) started.complete();
    };
    harness.advance({'first': harness.now});
    final oldFlush = harness.manager.flush();
    await started.future;
    harness.restart();
    harness.advance({'successor': harness.now});
    // Let the successor's queued journal save finish while the old ACK waits.
    for (var i = 0; i < 10; i++) {
      await Future<void>.delayed(Duration.zero);
    }
    gate.complete();
    await oldFlush;
    harness.restart();
    expect(harness.manager.getEffectiveTimestamp('successor'), harness.now);
    final saved = ReadStateStorage(harness.prefs).readJournal(
      '${harness.keys.public}:${Uri.encodeComponent('https://relay.example')}',
    )!;
    expect((saved['pending'] as Map).length, 1);
    await harness.manager.flush();
    expect(harness.manager.syncError, isNull);
    expect(harness.published, {'first': harness.now, 'successor': harness.now});
  });

  test('retired queued save cannot overwrite a successor save', () async {
    final harness = await _Owner.create(corpus['now']);
    await harness.manager.flush();
    final old = harness.manager;
    old.markContextRead('queued-old', harness.now);
    harness.restart();
    harness.advance({'successor': harness.now});
    await harness.manager.flush();
    harness.restart();
    expect(harness.manager.getEffectiveTimestamp('successor'), harness.now);
  });

  for (final count in [4095, 4096]) {
    test('owner preflights projected snapshot count from $count', () async {
      final harness = await _Owner.create(corpus['now']);
      harness.session.history.addAll(
        List.generate(count, (i) => harness.unrelated('other-$i', '')),
      );
      harness.advance({'room': harness.now});
      await harness.manager.flush();
      if (count == 4095) {
        expect(harness.manager.syncError, isNull);
        expect(harness.session.attempts.length, 1);
      } else {
        expect(harness.manager.syncError, isNotNull);
        expect(harness.session.attempts, isEmpty);
        harness.restart();
        expect(harness.manager.getEffectiveTimestamp('room'), harness.now);
      }
    });
  }

  for (final overflow in [false, true]) {
    test(
      'owner preflights actual encrypted array bytes overflow=$overflow',
      () async {
        final harness = await _Owner.create(corpus['now']);
        harness.advance({'room': harness.now});
        await harness.manager.flush();
        expect(harness.manager.syncError, isNull);
        final own = harness.session.retained.values.single;
        // Fill the complete snapshot to the exact byte boundary; retain the
        // existing coordinate so the next update is a replacement, not addition.
        final filler = harness.unrelated('filler', '');
        final base = utf8
            .encode(jsonEncode([filler.toJson(), own.toJson()]))
            .length;
        harness.session.history.add(
          harness.unrelated(
            'filler',
            'x' * (readStateSnapshotMaxBytes - base - (overflow ? 0 : 4096)),
          ),
        );
        final before = harness.session.attempts.length;
        harness.advance({'a-new-long-context-${'z' * 200}': harness.now});
        await harness.manager.flush();
        if (overflow) {
          expect(harness.manager.syncError, isNotNull);
          expect(harness.session.attempts.length, before);
        } else {
          expect(harness.manager.syncError, isNull);
          expect(harness.session.attempts.length, before + 1);
          expect(
            utf8
                .encode(
                  jsonEncode(
                    [
                      ...harness.session.history,
                      ...harness.session.retained.values,
                    ].map((e) => e.toJson()).toList(),
                  ),
                )
                .length,
            lessThanOrEqualTo(readStateSnapshotMaxBytes),
          );
        }
      },
    );
  }
  for (final vector in corpus['lifecycle'] as List) {
    test('shared owner lifecycle: ${vector['id']}', () async {
      final harness = await _Owner.create(corpus['now']);
      final id = vector['id'];
      if (id == 'old-event-new-frontier') {
        harness.remote(
          map(vector['frontiers']),
          createdAt: vector['eventCreatedAt'],
        );
        await harness.manager.initialize();
        expect(harness.manager.effectiveContexts, map(vector['frontiers']));
        return;
      }
      final frontiers = {
        for (var i = 0; i < 1000; i++)
          'thread:${i.toRadixString(16).padLeft(64, '0')}': harness.now - 10,
      };
      harness.advance(frontiers);
      // Inspect the actual journal at the network boundary, not after send.
      harness.session.beforePublish = (event) {
        final saved = ReadStateStorage(harness.prefs).readJournal(
          '${harness.keys.public}:${Uri.encodeComponent('https://relay.example')}',
        )!;
        final slot = event.tags
            .firstWhere((tag) => tag.first == 'd')[1]
            .substring(readStateDTagPrefix.length);
        expect(saved['slots'] as List, contains(slot));
        expect((saved['pending'] as Map)[slot], event.toJson());
        expect(saved['placement'] as List, isNotEmpty);
      };
      if (id == 'partial-publish-restart') harness.session.failAt = 2;
      await harness.manager.flush();
      if (id == 'partial-publish-restart') {
        expect(harness.manager.syncError, isNotNull);
        final unknown = harness.session.attempts.last;
        final saved = ReadStateStorage(harness.prefs).readJournal(
          '${harness.keys.public}:${Uri.encodeComponent('https://relay.example')}',
        )!;
        expect(saved['local'], frontiers);
        expect((saved['pending'] as Map).length, 1);
        harness.restart();
        harness.session.failAt = null;
        await harness.manager.flush();
        expect(harness.manager.syncError, isNull);
        expect(harness.session.attempts[2].toJson(), unknown.toJson());
        expect(harness.published, frontiers);
        return;
      }
      expect(harness.manager.syncError, isNull);
      final initial = Map<String, NostrEvent>.of(harness.session.retained);
      expect(initial.length, greaterThan(1));
      final first = initial.entries.first;
      final firstKeys = harness.decode(first.value).keys;
      if (id == 'unchanged-blob-not-republished') {
        final changedKey = firstKeys.first;
        harness.advance({changedKey: harness.now});
        final count = harness.session.attempts.length;
        await harness.manager.flush();
        expect(harness.session.attempts.length, count + 1);
        for (final entry in initial.entries.skip(1)) {
          expect(
            harness.session.retained[entry.key]!.toJson(),
            entry.value.toJson(),
          );
        }
      } else if (id == 'shrink-and-slot-reuse') {
        harness.now += corpus['retentionSeconds'] as int;
        harness.advance({for (final key in firstKeys) key: harness.now});
        await harness.manager.flush();
        expect(harness.manager.syncError, isNull);
        expect(
          harness.decode(harness.session.retained[first.key]!).keys,
          firstKeys,
        );
        for (final slot in initial.keys.skip(1)) {
          expect(harness.decode(harness.session.retained[slot]!), isEmpty);
        }
        harness.restart();
        harness.advance({
          for (var i = 0; i < 800; i++)
            'thread:new-${i.toRadixString(16).padLeft(64, '0')}': harness.now,
        });
        await harness.manager.flush();
        expect(harness.manager.syncError, isNull);
        expect(harness.session.retained.keys, containsAll(initial.keys));
        expect(
          harness.decode(harness.session.retained[first.key]!).keys,
          containsAll(firstKeys),
        );
        expect(
          harness.decode(harness.session.retained[initial.keys.elementAt(1)]!),
          isNotEmpty,
        );
      } else {
        fail('Unimplemented shared lifecycle obligation: $id');
      }
    });
  }
}

/// Real owner, real signer and crypto; only the network is replaced.
class _Owner {
  int now;
  final nostr.Keys keys;
  final SharedPreferences prefs;
  final ReadStateCrypto crypto;
  final _VectorRelay session = _VectorRelay();
  late ReadStateManager manager;

  _Owner(this.now, this.keys, this.prefs, this.crypto) {
    _open();
  }

  static Future<_Owner> create(int now, {_FailingJournalStore? store}) async {
    SharedPreferences.setMockInitialValues({});
    if (store != null) SharedPreferencesStorePlatform.instance = store;
    final keys = nostr.Keys.generate();
    final owner = _Owner(
      now,
      keys,
      await SharedPreferences.getInstance(),
      ReadStateCrypto.tryCreate(nsec: keys.nsec, pubkey: keys.public)!,
    );
    addTearDown(() => owner.manager.dispose(flushPending: false));
    return owner;
  }

  void _open() {
    manager = ReadStateManager(
      pubkey: keys.public,
      relayUrl: 'https://relay.example',
      prefs: prefs,
      crypto: crypto,
      relaySession: session,
      signedEventRelay: SignedEventRelay(session: session, nsec: keys.nsec),
      remoteEnabled: true,
      onChanged: () {},
      now: () => now,
    );
  }

  void restart() {
    manager.dispose(flushPending: false);
    _open();
  }

  void advance(Map<String, int> values) {
    for (final entry in values.entries) {
      manager.markContextRead(entry.key, entry.value);
    }
  }

  NostrEvent unrelated(String coordinate, String content) => NostrEvent(
    id: '0' * 64,
    pubkey: keys.public,
    createdAt: now,
    kind: EventKind.readState,
    tags: [
      ['d', coordinate],
    ],
    content: content,
    sig: '0' * 128,
  );

  void remote(Map<String, int> values, {int? createdAt}) {
    session.history.add(
      NostrEvent(
        id: 'peer-${session.history.length}',
        pubkey: keys.public,
        createdAt: createdAt ?? now,
        kind: EventKind.readState,
        tags: [
          ['d', '${readStateDTagPrefix}peer-${session.history.length}'],
          ['t', 'read-state'],
        ],
        content: crypto.encrypt(
          jsonEncode(
            ReadStateBlob(clientId: 'peer', contexts: values).toJson(),
          ),
        ),
        sig: 'transport-validated',
      ),
    );
  }

  Map<String, int> decode(NostrEvent event) =>
      decodeReadStateBlob(crypto.decrypt(event.content))!.contexts;

  Map<String, int> get published => mergeReadStateContexts(
    session.retained.values.map(
      (event) => decodeReadStateBlob(crypto.decrypt(event.content))!.contexts,
    ),
  );
}

class _VectorRelay extends RelaySessionNotifier {
  final history = <NostrEvent>[];
  final attempts = <NostrEvent>[];
  int? failAt;
  bool dropBeforeAccept = false;
  Completer<void>? ackGate;
  void Function(NostrEvent)? beforePublish;
  final retained = <String, NostrEvent>{};

  @override
  Future<List<NostrEvent>> fetchReadStateSnapshot({
    Duration timeout = const Duration(seconds: 8),
  }) async => [...history, ...retained.values];

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async => () {};

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    beforePublish?.call(event);
    attempts.add(event);
    if (dropBeforeAccept && attempts.length == failAt) {
      throw TimeoutException('packet never arrived');
    }
    final coordinate = event.tags.firstWhere((tag) => tag.first == 'd')[1];
    final old = retained[coordinate];
    if (old == null ||
        event.createdAt > old.createdAt ||
        (event.createdAt == old.createdAt && event.id.compareTo(old.id) < 0)) {
      retained[coordinate] = event;
    }
    if (attempts.length == failAt) throw TimeoutException('unknown outcome');
    await ackGate?.future;
    return event;
  }
}

class _FailingJournalStore extends InMemorySharedPreferencesStore {
  bool fail = true;
  int writes = 0;
  void Function(Object)? afterSave;
  _FailingJournalStore() : super.empty();
  @override
  Future<bool> setValue(String type, String key, Object value) async {
    if (key.startsWith('flutter.buzz.nip-rs.journal.v1:')) {
      writes++;
      if (fail) return Future.value(false);
    }
    final result = await super.setValue(type, key, value);
    if (key.startsWith('flutter.buzz.nip-rs.journal.v1:')) {
      afterSave?.call(value);
    }
    return result;
  }
}
