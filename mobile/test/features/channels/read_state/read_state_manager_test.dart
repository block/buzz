import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:buzz/shared/read_state/read_state_format.dart';
import 'package:buzz/shared/read_state/read_state_manager.dart';
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
