import 'dart:async';
import 'dart:convert';

import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:buzz/shared/read_state/read_state_format.dart';
import 'package:buzz/shared/read_state/read_state_manager.dart';
import 'package:buzz/shared/read_state/read_state_storage.dart';
import 'package:buzz/shared/read_state/read_state_time.dart';
import 'package:buzz/shared/relay/relay.dart';

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
      prefs: prefs,
      crypto: crypto!,
      relaySession: null,
      signedEventRelay: relay,
      remoteEnabled: true,
      onChanged: () {},
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
      prefs: prefs,
      crypto: crypto!,
      relaySession: null,
      signedEventRelay: relay,
      remoteEnabled: true,
      onChanged: () {},
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
        prefs: prefs,
        crypto: crypto!,
        relaySession: null,
        signedEventRelay: relay,
        remoteEnabled: true,
        onChanged: () {},
      );

      manager.markContextRead('channel-1', 42);
      await manager.flush();

      manager.markContextRead('channel-2', 43);
      await manager.flush();

      expect(relay.submitCount, 1);
      expect(manager.getEffectiveTimestamp('channel-2'), 43);
    },
  );

  test('caps a large slot instead of disabling sync', () async {
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
      prefs: prefs,
      crypto: crypto,
      relaySession: null,
      signedEventRelay: relay,
      remoteEnabled: true,
      onChanged: () {},
    );

    // About 100 KiB of message marks, more than NIP-44 can encrypt.
    for (var index = 0; index < 1400; index++) {
      manager.markContextRead(
        'msg:${index.toString().padLeft(64, '0')}',
        index + 1,
      );
    }
    manager.markContextRead('channel-1', 5);
    await manager.flush();

    manager.markContextRead('msg:new', 2000);
    await manager.flush();

    expect(relay.submitCount, 2);
    final plaintext = crypto.decrypt(relay.contents.last);
    expect(utf8.encode(plaintext).length, lessThan(readStatePlaintextBytes));
    final published = decodeReadStateBlob(plaintext)!.contexts;
    // Broad marks come first, then the newest message marks.
    expect(published['channel-1'], 5);
    expect(published['msg:new'], 2000);
    expect(published, isNot(contains('msg:${'0' * 64}')));
    // Marks left out of the slot stay in local state.
    expect(manager.getEffectiveTimestamp('msg:${'0' * 64}'), 1);
  });

  test('republishes merged broad marks but not message marks', () async {
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
      prefs: prefs,
      crypto: crypto,
      relaySession: session,
      signedEventRelay: relay,
      remoteEnabled: true,
      onChanged: () {},
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
    manager.markContextRead('msg:mine', 130);
    await manager.flush();

    // The web app may prune `msg:a` once `activity:` reads it, so this
    // device must not bring it back. It still reads it locally.
    expect(manager.getEffectiveTimestamp('msg:a'), 110);
    final published = decodeReadStateBlob(
      crypto.decrypt(relay.contents.last),
    )!.contexts;
    expect(published, {
      'channel-1': 100,
      'activity:channel-1': 120,
      'msg:mine': 130,
    });
  });

  test('publishes again when a debounce fires during a publish', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keychain = nostr.Keys.generate();
    final crypto = ReadStateCrypto.tryCreate(
      nsec: keychain.nsec,
      pubkey: keychain.public,
    )!;

    for (final failFirst in [false, true]) {
      fakeAsync((async) {
        final relay = _ParkedSignedEventRelay(failFirst: failFirst);
        final manager = ReadStateManager(
          pubkey: keychain.public,
          prefs: prefs,
          crypto: crypto,
          relaySession: null,
          signedEventRelay: relay,
          remoteEnabled: true,
          onChanged: () {},
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

  test('bounds saved marks across a restart', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keychain = nostr.Keys.generate();
    final crypto = ReadStateCrypto.tryCreate(
      nsec: keychain.nsec,
      pubkey: keychain.public,
    )!;
    ReadStateManager create() => ReadStateManager(
      pubkey: keychain.public,
      prefs: prefs,
      crypto: crypto,
      relaySession: null,
      signedEventRelay: null,
      remoteEnabled: false,
      onChanged: () {},
    );

    final now = currentUnixSeconds();
    final first = create();
    first.markContextRead('channel-1', now - 2 * readStateHorizonSeconds);
    first.markContextRead('msg:stale', now - 2 * readStateHorizonSeconds);
    for (var index = 0; index < localMaxPrunableContexts + 200; index++) {
      first.markContextRead('msg:$index', now - 1000 + index % 900);
    }
    // This session still reads every mark.
    expect(first.getEffectiveTimestamp('msg:stale'), isNotNull);
    first.dispose();

    final stored = ReadStateStorage(prefs).read(keychain.public);
    final messages = stored.contexts.keys.where((k) => k.startsWith('msg:'));
    expect(messages.length, localMaxPrunableContexts);
    expect(stored.contexts, isNot(contains('msg:stale')));
    expect(stored.contexts['channel-1'], now - 2 * readStateHorizonSeconds);
    // All three saved structures hold the same marks.
    expect(stored.publishableContextIds, stored.contexts.keys.toSet());
    expect(stored.sourceCreatedAt.keys.toSet(), stored.contexts.keys.toSet());

    final restarted = create();
    expect(restarted.getEffectiveTimestamp('msg:stale'), isNull);
    expect(
      restarted.getEffectiveTimestamp('channel-1'),
      now - 2 * readStateHorizonSeconds,
    );
    expect(
      restarted.effectiveContexts.keys.where((k) => k.startsWith('msg:')),
      hasLength(localMaxPrunableContexts),
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
    final clientId = storage.getOrCreateClientId(keychain.public);
    final slotId = storage.getOrCreateSlotId(keychain.public);
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
      prefs: prefs,
      crypto: crypto,
      relaySession: session,
      signedEventRelay: relay,
      remoteEnabled: true,
      onChanged: () {},
    );

    await manager.initialize();
    // About 100 KiB of message marks, so retention must leave some out.
    for (var index = 0; index < 1400; index++) {
      manager.markContextRead('msg:${index.toString().padLeft(64, '0')}', 200);
    }
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
      prefs: prefs,
      crypto: crypto,
      relaySession: null,
      signedEventRelay: restartedRelay,
      remoteEnabled: true,
      onChanged: () {},
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

  test('leaves the slot when carried override keys do not fit', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final keychain = nostr.Keys.generate();
    final crypto = ReadStateCrypto.tryCreate(
      nsec: keychain.nsec,
      pubkey: keychain.public,
    )!;
    final storage = ReadStateStorage(prefs);
    final clientId = storage.getOrCreateClientId(keychain.public);
    final slotId = storage.getOrCreateSlotId(keychain.public);
    final session = _FakeRelaySession()
      ..historyEvents = [
        _readStateEvent(
          pubkey: keychain.public,
          crypto: crypto,
          clientId: clientId,
          slotId: slotId,
          contexts: {
            for (var index = 0; index < 320; index++) ...{
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
      prefs: prefs,
      crypto: crypto,
      relaySession: session,
      signedEventRelay: relay,
      remoteEnabled: true,
      onChanged: () {},
    );

    await manager.initialize();
    manager.markContextRead('channel-1', 300);
    await manager.flush();

    expect(relay.submitCount, 0);
    expect(manager.getEffectiveTimestamp('channel-1'), 300);
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
      prefs: prefs,
      crypto: crypto!,
      relaySession: relay,
      signedEventRelay: _FakeSignedEventRelay(),
      remoteEnabled: true,
      onChanged: () {},
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
    void Function(NostrEvent event)? onSigned,
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
    void Function(NostrEvent event)? onSigned,
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
    void Function(NostrEvent event)? onSigned,
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
    void Function(NostrEvent event)? onSigned,
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
}) {
  final blob = ReadStateBlob(clientId: clientId, contexts: contexts);
  return NostrEvent(
    id: 'event-$clientId-$createdAt',
    pubkey: pubkey,
    createdAt: createdAt,
    kind: EventKind.readState,
    tags: [
      ['d', '$readStateDTagPrefix$slotId'],
      ['t', 'read-state'],
    ],
    content: crypto.encrypt(jsonEncode(blob.toJson())),
    sig: 'sig',
  );
}

class _FakeRelaySession extends RelaySessionNotifier {
  List<NostrEvent> historyEvents = [];

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => historyEvents;

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async => () {};
}
