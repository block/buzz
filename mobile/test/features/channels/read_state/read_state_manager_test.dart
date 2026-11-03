import 'dart:async';
import 'dart:convert';

import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:buzz/shared/read_state/read_state_format.dart';
import 'package:buzz/shared/read_state/read_state_manager.dart';
import 'package:buzz/shared/read_state/read_state_storage.dart';
import 'package:buzz/shared/relay/relay.dart';

const _relayUrl = 'https://relay.example';

void main() {
  test('dispose flushes a pending publish after marking disposed', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keychain = nostr.Keys.generate();
    final nsec = keychain.nsec;
    final crypto = ReadStateCrypto.tryCreate(
      nsec: nsec,
      pubkey: keychain.public,
    );
    final relay = _FakeSignedEventRelay();
    final manager = ReadStateManager(
      pubkey: keychain.public,
      relayUrl: _relayUrl,
      prefs: prefs,
      crypto: crypto!,
      relaySession: null,
      signedEventRelay: relay,
      remoteEnabled: true,
      onChanged: () {},
      now: () => 2000,
    );

    manager.markContextRead('channel-1', 42);
    manager.dispose();

    final submitted = await relay.submitted.future.timeout(
      const Duration(seconds: 1),
    );
    expect(submitted.kind, EventKind.readState);
    expect(
      submitted.tags.any(
        (tag) => tag.length == 2 && tag[0] == 't' && tag[1] == 'read-state',
      ),
      isTrue,
    );
  });

  test('disables remote sync after relay rejects read-state kind', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keychain = nostr.Keys.generate();
    final nsec = keychain.nsec;
    final crypto = ReadStateCrypto.tryCreate(
      nsec: nsec,
      pubkey: keychain.public,
    );
    final relay = _UnsupportedKindSignedEventRelay();
    final manager = ReadStateManager(
      pubkey: keychain.public,
      relayUrl: _relayUrl,
      prefs: prefs,
      crypto: crypto!,
      relaySession: null,
      signedEventRelay: relay,
      remoteEnabled: true,
      onChanged: () {},
      now: () => 2000,
    );

    manager.markContextRead('channel-1', 42);
    await manager.flush();

    manager.markContextRead('channel-2', 43);
    await manager.flush();

    expect(relay.submitCount, 1);
    expect(manager.getEffectiveTimestamp('channel-2'), 43);
  });

  test(
    'disables remote sync after token permanently lacks write scope',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final keychain = nostr.Keys.generate();
      final nsec = keychain.nsec;
      final crypto = ReadStateCrypto.tryCreate(
        nsec: nsec,
        pubkey: keychain.public,
      );
      final relay = _MissingScopeSignedEventRelay();
      final manager = ReadStateManager(
        pubkey: keychain.public,
        relayUrl: _relayUrl,
        prefs: prefs,
        crypto: crypto!,
        relaySession: null,
        signedEventRelay: relay,
        remoteEnabled: true,
        onChanged: () {},
        now: () => 2000,
      );

      manager.markContextRead('channel-1', 42);
      await manager.flush();

      manager.markContextRead('channel-2', 43);
      await manager.flush();

      expect(relay.submitCount, 1);
      expect(manager.getEffectiveTimestamp('channel-2'), 43);
    },
  );

  test('overflows a large frontier set instead of evicting', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keychain = nostr.Keys.generate();
    final crypto = ReadStateCrypto.tryCreate(
      nsec: keychain.nsec,
      pubkey: keychain.public,
    )!;
    final relay = _FakeSignedEventRelay();
    final manager = ReadStateManager(
      pubkey: keychain.public,
      relayUrl: _relayUrl,
      prefs: prefs,
      crypto: crypto,
      relaySession: null,
      signedEventRelay: relay,
      remoteEnabled: true,
      onChanged: () {},
      now: () => 2000,
    );

    // About 100 KiB of message marks, more than NIP-44 can encrypt.
    for (var index = 0; index < 1400; index++) {
      manager.markContextRead(
        'thread:${index.toString().padLeft(64, '0')}',
        index + 1,
      );
    }
    manager.markContextRead('channel-1', 5);
    await manager.flush();

    manager.markContextRead('thread:new', 2000);
    await manager.flush();

    expect(relay.submitCount, greaterThan(2));
    final published = mergeReadStateContexts([
      for (final content in relay.contents)
        decodeReadStateBlob(crypto.decrypt(content))!.contexts,
    ]);
    for (final content in relay.contents) {
      expect(
        utf8.encode(crypto.decrypt(content)).length,
        lessThanOrEqualTo(readStatePlaintextBytes),
      );
    }
    expect(published['channel-1'], 5);
    expect(published['thread:new'], 2000);
    expect(published['thread:${'0' * 64}'], 1);
    expect(published.length, 1402);
    manager.dispose(flushPending: false);
  });

  test('publishes locally advanced values, not merged peer maxima', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keychain = nostr.Keys.generate();
    final crypto = ReadStateCrypto.tryCreate(
      nsec: keychain.nsec,
      pubkey: keychain.public,
    )!;
    final session = _FakeRelaySession();
    final relay = _FakeSignedEventRelay();
    final manager = ReadStateManager(
      pubkey: keychain.public,
      relayUrl: _relayUrl,
      prefs: prefs,
      crypto: crypto,
      relaySession: session,
      signedEventRelay: relay,
      remoteEnabled: true,
      onChanged: () {},
      now: () => 2000,
    );
    session.historyEvents = [
      _readStateEvent(
        pubkey: keychain.public,
        crypto: crypto,
        clientId: 'web-client',
        slotId: 'web-slot',
        contexts: {'channel-1': 100, 'activity:channel-1': 120, 'msg:a': 110},
        createdAt: 100,
      ),
    ];

    await manager.initialize();
    manager.markContextRead('channel-1', 10);
    await manager.flush();

    // The web app may prune `msg:a` once `activity:` reads it, so this
    // device must not bring it back. It still reads it locally.
    expect(manager.getEffectiveTimestamp('msg:a'), 110);
    final published = decodeReadStateBlob(
      crypto.decrypt(relay.contents.last),
    )!.contexts;
    expect(published, {'channel-1': 10});
  });

  for (final ownSlot in [false, true]) {
    test(
      'unknown version blocks publication only for own slot: $ownSlot',
      () async {
        SharedPreferences.setMockInitialValues({});
        final prefs = await SharedPreferences.getInstance();
        final keychain = nostr.Keys.generate();
        final crypto = ReadStateCrypto.tryCreate(
          nsec: keychain.nsec,
          pubkey: keychain.public,
        )!;
        final slot = ReadStateStorage(
          prefs,
        ).getOrCreateSlotId(keychain.public, _relayUrl);
        final session = _FakeRelaySession()
          ..historyEvents = [
            _readStateEvent(
              pubkey: keychain.public,
              crypto: crypto,
              clientId: 'future-client',
              slotId: ownSlot ? slot : 'f' * 32,
              contexts: {'channel-1': 100},
              createdAt: 100,
              version: 2,
            ),
          ];
        final relay = _FakeSignedEventRelay();
        final manager = ReadStateManager(
          pubkey: keychain.public,
          relayUrl: _relayUrl,
          prefs: prefs,
          crypto: crypto,
          relaySession: session,
          signedEventRelay: relay,
          remoteEnabled: true,
          onChanged: () {},
          now: () => 2000,
        );
        manager.markContextRead('channel-1', 10);
        await manager.flush();
        expect(manager.getEffectiveTimestamp('channel-1'), 10);
        expect(relay.contents, ownSlot ? isEmpty : hasLength(1));
        expect(manager.syncError, ownSlot ? isStateError : isNull);
        if (!ownSlot) {
          expect(
            decodeReadStateBlob(
              crypto.decrypt(relay.contents.single),
            )!.contexts,
            {'channel-1': 10},
          );
        }
        manager.dispose(flushPending: false);
      },
    );
  }

  test(
    'upgrade carries legacy publishable frontiers into the journal',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final keychain = nostr.Keys.generate();
      final crypto = ReadStateCrypto.tryCreate(
        nsec: keychain.nsec,
        pubkey: keychain.public,
      )!;
      ReadStateStorage(prefs).write(
        keychain.public,
        _relayUrl,
        {'channel-1': 10, 'thread:root': 20, 'msg:a': 30, 'seeded': 40},
        {'channel-1', 'thread:root', 'msg:a'},
        {},
      );
      final relay = _FakeSignedEventRelay();
      ReadStateManager create() => ReadStateManager(
        pubkey: keychain.public,
        relayUrl: _relayUrl,
        prefs: prefs,
        crypto: crypto,
        relaySession: null,
        signedEventRelay: relay,
        remoteEnabled: true,
        onChanged: () {},
        now: () => 2000,
      );
      final manager = create();
      await manager.flush();
      expect(
        decodeReadStateBlob(crypto.decrypt(relay.contents.single))!.contexts,
        {'channel-1': 10, 'thread:root': 20},
      );
      manager.dispose(flushPending: false);
      final restarted = create();
      expect(restarted.getEffectiveTimestamp('msg:a'), 30);
      expect(restarted.getEffectiveTimestamp('seeded'), 40);
      restarted.markContextRead('new-channel', 50);
      await restarted.flush();
      expect(
        decodeReadStateBlob(crypto.decrypt(relay.contents.last))!.contexts,
        {'channel-1': 10, 'thread:root': 20, 'new-channel': 50},
      );
      restarted.dispose(flushPending: false);
    },
  );

  test('publishes again when a debounce fires during a publish', () async {
    final keychain = nostr.Keys.generate();
    final crypto = ReadStateCrypto.tryCreate(
      nsec: keychain.nsec,
      pubkey: keychain.public,
    )!;

    for (final failFirst in [false, true]) {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      fakeAsync((async) {
        final relay = _ParkedSignedEventRelay(failFirst: failFirst);
        final manager = ReadStateManager(
          pubkey: keychain.public,
          relayUrl: _relayUrl,
          prefs: prefs,
          crypto: crypto,
          relaySession: null,
          signedEventRelay: relay,
          remoteEnabled: true,
          onChanged: () {},
          now: () => 2000,
        );

        manager.markContextRead('a-$failFirst', 10);
        async.elapse(const Duration(seconds: 5));
        // Publish A has its snapshot and waits on the relay.
        expect(relay.contents, hasLength(1));

        manager.markContextRead('b-$failFirst', 20);
        async.elapse(const Duration(seconds: 5));
        expect(relay.contents, hasLength(1));

        relay.release();
        async.flushMicrotasks();

        expect(relay.contents, hasLength(2), reason: 'failFirst=$failFirst');
        final second = decodeReadStateBlob(
          crypto.decrypt(relay.contents.last),
        )!.contexts;
        expect(second['b-$failFirst'], 20);
        expect(second['a-$failFirst'], 10);
        manager.dispose(flushPending: false);
      });
    }
  });

  test(
    'retains recent frontiers without a count eviction across restart',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final keys = nostr.Keys.generate();
      final crypto = ReadStateCrypto.tryCreate(
        nsec: keys.nsec,
        pubkey: keys.public,
      )!;
      const now = 1800000000;
      ReadStateManager create() => ReadStateManager(
        pubkey: keys.public,
        relayUrl: 'https://relay.example',
        prefs: prefs,
        crypto: crypto,
        relaySession: null,
        signedEventRelay: _FakeSignedEventRelay(),
        remoteEnabled: true,
        onChanged: () {},
        now: () => now,
      );
      final first = create();
      first.markContextRead('old', now - readStateHorizonSeconds - 1);
      for (var i = 0; i < 1200; i++) {
        first.markContextRead('thread:$i', now - 10);
      }
      await first.flush();
      first.dispose(flushPending: false);
      final restarted = create();
      expect(restarted.getEffectiveTimestamp('old'), isNull);
      expect(restarted.effectiveContexts.length, 1200);
      restarted.dispose(flushPending: false);
    },
  );

  test('each community keeps its own slot, seeded from the shared one, '
      'and none takes the shared marks', () async {
    SharedPreferences.setMockInitialValues({
      'buzz.nip-rs.slot-id:pk': 'a' * 32,
      'buzz.channel-read-state.v2:pk':
          '{"channel-1":"2026-01-01T00:00:00.000",'
          '"thread-activity:root-1":"2026-01-01T00:00:00.000"}',
      'buzz.channel-read-state.publishable.v1:pk':
          '["channel-1","thread-activity:root-1"]',
    });
    final prefs = await SharedPreferences.getInstance();
    final storage = ReadStateStorage(prefs);

    // Both communities keep publishing to the slot they already have.
    expect(storage.getOrCreateSlotId('pk', _relayUrl), 'a' * 32);
    expect(storage.getOrCreateSlotId('pk', 'https://other.example'), 'a' * 32);
    storage.writeSlotId('pk', 'https://other.example', 'b' * 32);
    expect(storage.getOrCreateSlotId('pk', _relayUrl), 'a' * 32);

    // The shared marks may be another community's, and the relay keeps
    // the published ones, so neither community takes them.
    expect(storage.read('pk', _relayUrl).contexts, isEmpty);
    expect(storage.read('pk', 'https://other.example').contexts, isEmpty);
    expect(prefs.containsKey('buzz.channel-read-state.v2:pk'), isFalse);
    expect(
      prefs.containsKey('buzz.channel-read-state.publishable.v1:pk'),
      isFalse,
    );
  });

  test('carries its own override keys and takes none from others', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keychain = nostr.Keys.generate();
    final crypto = ReadStateCrypto.tryCreate(
      nsec: keychain.nsec,
      pubkey: keychain.public,
    )!;
    final storage = ReadStateStorage(prefs);
    final clientId = storage.getOrCreateClientId(keychain.public, _relayUrl);
    final slotId = storage.getOrCreateSlotId(keychain.public, _relayUrl);
    const ownGroup = {
      'ov_s:channel-1': 2,
      'ov_c:channel-1': 1,
      'ov_b:channel-1': 90,
    };
    final oldMessage = 'msg:${'d' * 64}';
    final oldMessageGroup = {'ov_c:$oldMessage': 96};
    final session = _FakeRelaySession()
      ..historyEvents = [
        _readStateEvent(
          pubkey: keychain.public,
          crypto: crypto,
          clientId: clientId,
          slotId: slotId,
          // `ov_s:channel-9` alone is a partial group, rejected whole.
          contexts: {
            'channel-1': 90,
            ...ownGroup,
            'ov_s:channel-9': 3,
            // Older than the local save horizon, so saving prunes it unless
            // its group protects it.
            oldMessage: 95,
            ...oldMessageGroup,
          },
          createdAt: 100,
        ),
        _readStateEvent(
          pubkey: keychain.public,
          crypto: crypto,
          clientId: 'web-client',
          slotId: 'web-slot',
          contexts: {'ov_c:channel-2': 4, 'esc:ov_x': 5, 'channel-2': 80},
          createdAt: 100,
        ),
      ];
    final relay = _FakeSignedEventRelay();
    final manager = ReadStateManager(
      pubkey: keychain.public,
      relayUrl: _relayUrl,
      prefs: prefs,
      crypto: crypto,
      relaySession: session,
      signedEventRelay: relay,
      remoteEnabled: true,
      onChanged: () {},
      now: () => 2000,
    );

    await manager.initialize();
    // Local message feedback never schedules wire writes. A frontier advance
    // triggers publication and must preserve complete own override groups.
    for (var index = 0; index < 1400; index++) {
      manager.markContextRead('msg:${index.toString().padLeft(64, '0')}', 200);
    }
    manager.markContextRead('channel-3', 200);
    await manager.flush();

    final published = decodeReadStateBlob(
      crypto.decrypt(relay.contents.last),
    )!.contexts;
    expect(published, containsPair('ov_s:channel-1', 2));
    expect(published, containsPair('ov_c:channel-1', 1));
    expect(published, containsPair('ov_b:channel-1', 90));
    // The group's frontier travels with it.
    expect(published, containsPair('channel-1', 90));
    expect(published, isNot(contains('ov_s:channel-9')));
    expect(published, isNot(contains('ov_c:channel-2')));
    expect(published, isNot(contains('esc:ov_x')));
    expect(manager.getEffectiveTimestamp('ov_s:channel-1'), isNull);
    manager.dispose(flushPending: false);

    // A restart without the relay still carries the group.
    final restartedRelay = _FakeSignedEventRelay();
    final restarted = ReadStateManager(
      pubkey: keychain.public,
      relayUrl: _relayUrl,
      prefs: prefs,
      crypto: crypto,
      relaySession: null,
      signedEventRelay: restartedRelay,
      remoteEnabled: true,
      onChanged: () {},
      now: () => 2000,
    );
    restarted.markContextRead('channel-3', 300);
    await restarted.flush();
    final republished = decodeReadStateBlob(
      crypto.decrypt(restartedRelay.contents.last),
    )!.contexts;
    for (final entry in ownGroup.entries) {
      expect(republished, containsPair(entry.key, entry.value));
    }
    expect(republished, containsPair('channel-1', 90));
    expect(republished, containsPair(oldMessage, 95));
    expect(republished, containsPair('ov_c:$oldMessage', 96));
    expect(republished, isNot(contains('ov_s:channel-9')));
    expect(republished, isNot(contains('ov_c:channel-2')));
  });

  test('rejects own-slot override groups that are malformed or have no '
      'frontier', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keychain = nostr.Keys.generate();
    final crypto = ReadStateCrypto.tryCreate(
      nsec: keychain.nsec,
      pubkey: keychain.public,
    )!;
    final storage = ReadStateStorage(prefs);
    final clientId = storage.getOrCreateClientId(keychain.public, _relayUrl);
    final slotId = storage.getOrCreateSlotId(keychain.public, _relayUrl);
    final session = _FakeRelaySession()
      ..historyEvents = [
        _readStateEvent(
          pubkey: keychain.public,
          crypto: crypto,
          clientId: clientId,
          slotId: slotId,
          contexts: const {
            // Its ov_b: sibling is invalid, so this is not a tombstone.
            'bad-sibling': 100,
            'ov_c:bad-sibling': 7,
            // Complete counters, but no frontier for `no-frontier`.
            'ov_s:no-frontier': 2,
            'ov_c:no-frontier': 1,
            'ov_b:no-frontier': 50,
            'channel-1': 90,
            'ov_s:channel-1': 2,
            'ov_c:channel-1': 1,
            'ov_b:channel-1': 90,
          },
          rawContexts: const {'ov_b:bad-sibling': 'invalid'},
          createdAt: 100,
        ),
      ];
    final relay = _FakeSignedEventRelay();
    final manager = ReadStateManager(
      pubkey: keychain.public,
      relayUrl: _relayUrl,
      prefs: prefs,
      crypto: crypto,
      relaySession: session,
      signedEventRelay: relay,
      remoteEnabled: true,
      onChanged: () {},
      now: () => 2000,
    );

    await manager.initialize();
    manager.markContextRead('channel-2', 300);
    await manager.flush();

    final published = decodeReadStateBlob(
      crypto.decrypt(relay.contents.last),
    )!.contexts;
    expect(published, {
      // Frontiers stay, including the one whose group was rejected.
      'bad-sibling': 100,
      'channel-1': 90,
      'channel-2': 300,
      'ov_s:channel-1': 2,
      'ov_c:channel-1': 1,
      'ov_b:channel-1': 90,
    });
    manager.dispose(flushPending: false);
  });

  test('leaves the slot when carried override keys do not fit', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keychain = nostr.Keys.generate();
    final crypto = ReadStateCrypto.tryCreate(
      nsec: keychain.nsec,
      pubkey: keychain.public,
    )!;
    final storage = ReadStateStorage(prefs);
    final clientId = storage.getOrCreateClientId(keychain.public, _relayUrl);
    final slotId = storage.getOrCreateSlotId(keychain.public, _relayUrl);
    final session = _FakeRelaySession()
      ..historyEvents = [
        _readStateEvent(
          pubkey: keychain.public,
          crypto: crypto,
          clientId: clientId,
          slotId: slotId,
          contexts: {
            for (var index = 0; index < 320; index++) ...{
              index.toString().padLeft(36, 'c'): 100,
              'ov_s:${index.toString().padLeft(36, 'c')}': 2,
              'ov_c:${index.toString().padLeft(36, 'c')}': 1,
              'ov_b:${index.toString().padLeft(36, 'c')}': 100,
            },
          },
          createdAt: 100,
        ),
      ];
    final relay = _FakeSignedEventRelay();
    final manager = ReadStateManager(
      pubkey: keychain.public,
      relayUrl: _relayUrl,
      prefs: prefs,
      crypto: crypto,
      relaySession: session,
      signedEventRelay: relay,
      remoteEnabled: true,
      onChanged: () {},
      now: () => 2000,
    );

    await manager.initialize();
    manager.markContextRead('channel-1', 300);
    await manager.flush();

    expect(relay.submitCount, 0);
    expect(manager.getEffectiveTimestamp('channel-1'), 300);
  });

  test(
    'partial publication survives restart and retries exact signed bytes',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final keys = nostr.Keys.generate();
      final crypto = ReadStateCrypto.tryCreate(
        nsec: keys.nsec,
        pubkey: keys.public,
      )!;
      final session = _JournalRelaySession();
      ReadStateManager create() => ReadStateManager(
        pubkey: keys.public,
        relayUrl: 'https://relay.example',
        prefs: prefs,
        crypto: crypto,
        relaySession: session,
        signedEventRelay: SignedEventRelay(session: session, nsec: keys.nsec),
        remoteEnabled: true,
        onChanged: () {},
        now: () => 1800000000,
      );
      final manager = create();
      for (var i = 0; i < 1100; i++) {
        manager.markContextRead(
          'thread:${i.toRadixString(16).padLeft(64, '0')}',
          1799999900,
        );
      }
      await manager.flush();
      expect(manager.syncError, isNotNull);
      expect(session.attempts, hasLength(2));
      final unknown = session.attempts.last;
      final saved = ReadStateStorage(prefs).readJournal(
        '${keys.public}:${Uri.encodeComponent('https://relay.example')}',
      )!;
      expect((saved['pending'] as Map).length, 1);
      expect((saved['local'] as Map).length, 1100);
      manager.dispose(flushPending: false);

      session.failSecond = false;
      final restarted = create();
      await restarted.flush();
      expect(restarted.syncError, isNull);
      expect(session.attempts[2].toJson(), unknown.toJson());
      expect(
        (ReadStateStorage(prefs).readJournal(
              '${keys.public}:${Uri.encodeComponent('https://relay.example')}',
            )!['pending']
            as Map),
        isEmpty,
      );
      final published = mergeReadStateContexts([
        for (final event in session.retained.values)
          decodeReadStateBlob(crypto.decrypt(event.content))!.contexts,
      ]);
      expect(published.length, 1100);
      final before = session.attempts.length;
      await restarted.flush();
      expect(session.attempts.length, before);
      restarted.dispose(flushPending: false);
    },
  );

  test('message feedback survives restart but is never published', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keys = nostr.Keys.generate();
    final crypto = ReadStateCrypto.tryCreate(
      nsec: keys.nsec,
      pubkey: keys.public,
    )!;
    final relay = _FakeSignedEventRelay();
    ReadStateManager create() => ReadStateManager(
      pubkey: keys.public,
      relayUrl: 'https://relay.example',
      prefs: prefs,
      crypto: crypto,
      relaySession: null,
      signedEventRelay: relay,
      remoteEnabled: true,
      onChanged: () {},
      now: () => 2000,
    );
    final manager = create();
    manager.markContextRead('msg:one', 100);
    manager.markContextRead('room', 90);
    await manager.flush();
    manager.dispose(flushPending: false);
    final restarted = create();
    expect(restarted.getEffectiveTimestamp('msg:one'), 100);
    for (final content in relay.contents) {
      expect(decodeReadStateBlob(crypto.decrypt(content))!.contexts, {
        'room': 90,
      });
    }
    restarted.dispose(flushPending: false);
  });

  test('signing waits for durable journal callback before sending', () async {
    final keys = nostr.Keys.generate();
    final session = _JournalRelaySession()..failSecond = false;
    final gate = Completer<void>();
    final relay = SignedEventRelay(session: session, nsec: keys.nsec);
    final send = relay.submit(
      kind: EventKind.readState,
      content: 'test',
      tags: const [],
      onSigned: (_) => gate.future,
    );
    await Future<void>.delayed(Duration.zero);
    expect(session.attempts, isEmpty);
    gate.completeError(StateError('disk failed'));
    await expectLater(send, throwsStateError);
    expect(session.attempts, isEmpty);
  });

  test('pending journal is scoped to its relay origin', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keys = nostr.Keys.generate();
    final crypto = ReadStateCrypto.tryCreate(
      nsec: keys.nsec,
      pubkey: keys.public,
    )!;
    ReadStateManager create(String origin) => ReadStateManager(
      pubkey: keys.public,
      relayUrl: origin,
      prefs: prefs,
      crypto: crypto,
      relaySession: null,
      signedEventRelay: _FakeSignedEventRelay(),
      remoteEnabled: true,
      onChanged: () {},
      now: () => 2000,
    );
    final first = create('https://first.example');
    first.markContextRead('first-only', 100);
    await first.flush();
    first.dispose(flushPending: false);
    final second = create('https://second.example');
    expect(second.getEffectiveTimestamp('first-only'), isNull);
    second.dispose(flushPending: false);
  });

  test(
    'a connect during a successful initialize does not sync again',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final keychain = nostr.Keys.generate();
      final crypto = ReadStateCrypto.tryCreate(
        nsec: keychain.nsec,
        pubkey: keychain.public,
      )!;
      // First connect: initialize waits on a history REQ held for the socket
      // while the session reports connected and the provider's listener fires.
      final gate = Completer<void>();
      final relay = _FakeRelaySession()..fetchGate = gate.future;
      final manager = ReadStateManager(
        pubkey: keychain.public,
        relayUrl: _relayUrl,
        prefs: prefs,
        crypto: crypto,
        relaySession: relay,
        signedEventRelay: _FakeSignedEventRelay(),
        remoteEnabled: true,
        onChanged: () {},
      );

      final initializing = manager.initialize();
      final reinitializing = manager.reinitializeRemote();
      relay.fetchGate = null;
      gate.complete();
      await Future.wait([initializing, reinitializing]);

      expect(relay.fetchCount, 1);
      expect(relay.subscribeCount, 1);
      expect(relay.liveSubscriptions, 1);

      // A later reconnect syncs again and still leaves one subscription.
      await manager.reinitializeRemote();
      expect(relay.fetchCount, 2);
      expect(relay.subscribeCount, 2);
      expect(relay.liveSubscriptions, 1);

      manager.dispose();
      expect(relay.liveSubscriptions, 0);
    },
  );

  test('a connect during a failed initialize syncs again', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keychain = nostr.Keys.generate();
    final crypto = ReadStateCrypto.tryCreate(
      nsec: keychain.nsec,
      pubkey: keychain.public,
    )!;
    final gate = Completer<void>();
    final relay = _FakeRelaySession()
      ..fetchGate = gate.future
      ..failNextFetch = true;
    final manager = ReadStateManager(
      pubkey: keychain.public,
      relayUrl: _relayUrl,
      prefs: prefs,
      crypto: crypto,
      relaySession: relay,
      signedEventRelay: _FakeSignedEventRelay(),
      remoteEnabled: true,
      onChanged: () {},
    );

    final initializing = manager.initialize();
    final reinitializing = manager.reinitializeRemote();
    relay.fetchGate = null;
    gate.complete();
    await Future.wait([initializing, reinitializing]);

    // Both passes subscribed, but the later one replaced the earlier one.
    expect(relay.fetchCount, 2);
    expect(relay.subscribeCount, 2);
    expect(relay.liveSubscriptions, 1);

    manager.dispose();
    expect(relay.liveSubscriptions, 0);
  });

  test('reconnects during a re-sync leave one live subscription', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keychain = nostr.Keys.generate();
    final crypto = ReadStateCrypto.tryCreate(
      nsec: keychain.nsec,
      pubkey: keychain.public,
    )!;
    final relay = _FakeRelaySession();
    final manager = ReadStateManager(
      pubkey: keychain.public,
      relayUrl: _relayUrl,
      prefs: prefs,
      crypto: crypto,
      relaySession: relay,
      signedEventRelay: _FakeSignedEventRelay(),
      remoteEnabled: true,
      onChanged: () {},
    );
    await manager.initialize();

    // The first re-sync holds its fetch; three more connects arrive.
    final gate = Completer<void>();
    relay.fetchGate = gate.future;
    final first = manager.reinitializeRemote();
    await Future<void>.delayed(Duration.zero);
    final later = [
      manager.reinitializeRemote(),
      manager.reinitializeRemote(),
      manager.reinitializeRemote(),
    ];
    relay.fetchGate = null;
    gate.complete();
    await Future.wait([first, ...later]);

    // One initial pass, the held re-sync, and one shared re-sync after it.
    expect(relay.fetchCount, 3);
    expect(relay.subscribeCount, 3);
    expect(relay.liveSubscriptions, 1);

    manager.dispose();
    expect(relay.liveSubscriptions, 0);
  });

  test('remote read-state rollback is ignored', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keychain = nostr.Keys.generate();
    final crypto = ReadStateCrypto.tryCreate(
      nsec: keychain.nsec,
      pubkey: keychain.public,
    );
    final relay = _FakeRelaySession();
    final manager = ReadStateManager(
      pubkey: keychain.public,
      relayUrl: _relayUrl,
      prefs: prefs,
      crypto: crypto!,
      relaySession: relay,
      signedEventRelay: _FakeSignedEventRelay(),
      remoteEnabled: true,
      onChanged: () {},
      now: () => 2000,
    );

    relay.historyEvents = [
      _readStateEvent(
        pubkey: keychain.public,
        crypto: crypto,
        clientId: 'remote-client',
        slotId: 'remote-slot',
        contexts: {'channel-1': 100},
        createdAt: 100,
      ),
      _readStateEvent(
        pubkey: keychain.public,
        crypto: crypto,
        clientId: 'remote-client',
        slotId: 'remote-slot',
        contexts: {'channel-1': 50},
        createdAt: 110,
      ),
    ];

    await manager.initialize();

    expect(manager.getEffectiveTimestamp('channel-1'), 100);
  });
}

class _SubmittedEvent {
  final int kind;
  final List<List<String>> tags;

  const _SubmittedEvent({required this.kind, required this.tags});
}

/// Build a stub NostrEvent for tests that just need a "ack" return value.
NostrEvent _stubAckEvent() => const NostrEvent(
  id: 'stub',
  pubkey: '',
  createdAt: 0,
  kind: 0,
  tags: [],
  content: '',
  sig: '',
);

class _FakeSignedEventRelay implements SignedEventRelay {
  final Completer<_SubmittedEvent> submitted = Completer<_SubmittedEvent>();
  final List<String> contents = [];
  int submitCount = 0;

  @override
  String? get pubkey => null;

  @override
  Future<NostrEvent> submit({
    required int kind,
    required String content,
    required List<List<String>> tags,
    int? createdAt,
    FutureOr<void> Function(NostrEvent event)? onSigned,
  }) async {
    submitCount++;
    contents.add(content);
    if (!submitted.isCompleted) {
      submitted.complete(_SubmittedEvent(kind: kind, tags: tags));
    }
    return _stubAckEvent();
  }
}

/// Holds the first submit until [release], then fails it if [failFirst].
class _ParkedSignedEventRelay implements SignedEventRelay {
  _ParkedSignedEventRelay({required this.failFirst});

  final bool failFirst;
  final List<String> contents = [];
  final Completer<void> _parked = Completer<void>();

  void release() => _parked.complete();

  @override
  String? get pubkey => null;

  @override
  Future<NostrEvent> submit({
    required int kind,
    required String content,
    required List<List<String>> tags,
    int? createdAt,
    FutureOr<void> Function(NostrEvent event)? onSigned,
  }) async {
    contents.add(content);
    if (contents.length == 1) {
      await _parked.future;
      if (failFirst) throw Exception('relay timeout');
    }
    return _stubAckEvent();
  }
}

class _UnsupportedKindSignedEventRelay implements SignedEventRelay {
  int submitCount = 0;

  @override
  String? get pubkey => null;

  @override
  Future<NostrEvent> submit({
    required int kind,
    required String content,
    required List<List<String>> tags,
    int? createdAt,
    FutureOr<void> Function(NostrEvent event)? onSigned,
  }) async {
    submitCount++;
    throw Exception('restricted: unknown event kind');
  }
}

class _MissingScopeSignedEventRelay implements SignedEventRelay {
  int submitCount = 0;

  @override
  String? get pubkey => null;

  @override
  Future<NostrEvent> submit({
    required int kind,
    required String content,
    required List<List<String>> tags,
    int? createdAt,
    FutureOr<void> Function(NostrEvent event)? onSigned,
  }) async {
    submitCount++;
    throw Exception('missing users:write');
  }
}

NostrEvent _readStateEvent({
  required String pubkey,
  required ReadStateCrypto crypto,
  required String clientId,
  required String slotId,
  required Map<String, int> contexts,
  required int createdAt,
  // Extra raw wire entries, such as values that are not valid timestamps.
  Map<String, Object?> rawContexts = const {},
  int version = 1,
}) {
  final blob = ReadStateBlob(clientId: clientId, contexts: contexts).toJson();
  blob['v'] = version;
  blob['contexts'] = <String, Object?>{...contexts, ...rawContexts};
  return NostrEvent(
    id: 'event-$clientId-$createdAt',
    pubkey: pubkey,
    createdAt: createdAt,
    kind: EventKind.readState,
    tags: [
      ['d', '$readStateDTagPrefix$slotId'],
      ['t', 'read-state'],
    ],
    content: crypto.encrypt(jsonEncode(blob)),
    sig: 'sig',
  );
}

class _FakeRelaySession extends RelaySessionNotifier {
  List<NostrEvent> historyEvents = [];
  Future<void>? fetchGate;
  bool failNextFetch = false;
  int fetchCount = 0;
  int subscribeCount = 0;
  int liveSubscriptions = 0;
  final List<NostrFilter> fetchFilters = [];
  final List<NostrFilter> subscribeFilters = [];

  @override
  Future<List<NostrEvent>> fetchReadStateSnapshot({
    Duration timeout = const Duration(seconds: 8),
  }) => fetchHistory(const NostrFilter(kinds: [30078]));

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    fetchCount++;
    fetchFilters.add(filter);
    await fetchGate;
    if (failNextFetch) {
      failNextFetch = false;
      throw TimeoutException('history timed out');
    }
    return historyEvents;
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    subscribeCount++;
    subscribeFilters.add(filter);
    liveSubscriptions++;
    var open = true;
    return () {
      if (open) liveSubscriptions--;
      open = false;
    };
  }
}

class _JournalRelaySession extends _FakeRelaySession {
  final attempts = <NostrEvent>[];
  final retained = <String, NostrEvent>{};
  bool failSecond = true;

  @override
  Future<List<NostrEvent>> fetchReadStateSnapshot({
    Duration timeout = const Duration(seconds: 8),
  }) async => retained.values.toList();

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    attempts.add(event);
    final slot = event.tags.firstWhere((tag) => tag.first == 'd')[1];
    retained[slot] = event;
    if (failSecond && attempts.length == 2) {
      throw TimeoutException('Accepted but acknowledgement lost');
    }
    return event;
  }
}
