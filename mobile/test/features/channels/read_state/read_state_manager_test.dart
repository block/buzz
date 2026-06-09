import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:sprout_mobile/features/channels/read_state/read_state_format.dart';
import 'package:sprout_mobile/features/channels/read_state/read_state_manager.dart';
import 'package:sprout_mobile/shared/relay/relay.dart';

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

  @override
  String? get pubkey => null;

  @override
  Future<NostrEvent> submit({
    required int kind,
    required String content,
    required List<List<String>> tags,
    int? createdAt,
  }) async {
    submitted.complete(_SubmittedEvent(kind: kind, tags: tags));
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
