import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/channels/channel_sort/channel_sort_manager.dart';
import 'package:buzz/features/channels/channel_sort/channel_sort_storage.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  late SharedPreferences prefs;
  late nostr.Keys keys;
  late ChannelSortCrypto crypto;

  setUp(() async {
    SharedPreferences.setMockInitialValues({});
    prefs = await SharedPreferences.getInstance();
    keys = nostr.Keys.generate();
    crypto = ChannelSortCrypto(keys.nsec, keys.public);
  });

  NostrEvent event(
    Map<String, String> groups,
    int createdAt, {
    String id = 'e',
  }) => NostrEvent(
    id: id,
    pubkey: keys.public,
    createdAt: createdAt,
    kind: EventKind.readState,
    tags: const [
      ['d', 'channel-sort'],
    ],
    content: crypto.encrypt(jsonEncode({'version': 1, 'groups': groups})),
    sig: 'sig',
  );

  ChannelSortManager manager(
    _FakeRelaySession relay,
    _RecordingSignedEventRelay signed, {
    Duration retry = const Duration(milliseconds: 5),
  }) => ChannelSortManager(
    pubkey: keys.public,
    relayUrl: 'wss://relay.example',
    prefs: prefs,
    crypto: crypto,
    relaySession: relay,
    signedEventRelay: signed,
    remoteEnabled: true,
    onChanged: () {},
    startupRetryBaseDelay: retry,
    publishDelay: const Duration(milliseconds: 5),
  );

  test(
    'adopts desktop payload and defaults unspecified groups to alpha',
    () async {
      final relay = _FakeRelaySession()
        ..historyEvents = [
          event({'channels': 'recent', 'section:abc': 'recent'}, 100),
        ];
      final subject = manager(relay, _RecordingSignedEventRelay());
      await subject.initialize();
      expect(subject.sortModeFor('channels'), ChannelSortMode.recent);
      expect(subject.sortModeFor('section:abc'), ChannelSortMode.recent);
      expect(subject.sortModeFor('dms'), ChannelSortMode.alpha);
      subject.dispose();
    },
  );

  test('publishes local edit using desktop encrypted wire format', () async {
    final relay = _FakeRelaySession();
    final signed = _RecordingSignedEventRelay();
    final subject = manager(relay, signed);
    await subject.initialize();
    subject.setSortModeFor('dms', ChannelSortMode.recent);
    final submitted = await signed.submitted.future.timeout(
      const Duration(seconds: 1),
    );
    expect(
      submitted.tags.any((tag) => tag[0] == 'd' && tag[1] == 'channel-sort'),
      isTrue,
    );
    final payload =
        jsonDecode(crypto.decrypt(submitted.content)) as Map<String, dynamic>;
    expect(payload, {
      'version': 1,
      'groups': {'dms': 'recent'},
    });
    subject.dispose();
  });

  test(
    'failed publish preflight retries without submitting stale state',
    () async {
      final relay = _FakeRelaySession();
      final signed = _RecordingSignedEventRelay();
      final subject = manager(relay, signed);
      await subject.initialize();

      relay.fetchFailures = 1;
      subject.setSortModeFor('dms', ChannelSortMode.recent);
      await Future<void>.delayed(const Duration(milliseconds: 8));
      expect(signed.submitCount, 0);

      final submitted = await signed.submitted.future.timeout(
        const Duration(seconds: 1),
      );
      expect(relay.fetchCount, greaterThanOrEqualTo(4));
      final payload =
          jsonDecode(crypto.decrypt(submitted.content)) as Map<String, dynamic>;
      expect(payload['groups'], {'dms': 'recent'});
      subject.dispose();
    },
  );

  test(
    'pending local edit survives manager rebuild and older relay state',
    () async {
      final firstRelay = _FakeRelaySession();
      final first = manager(firstRelay, _RecordingSignedEventRelay());
      await first.initialize();
      first.setSortModeFor('channels', ChannelSortMode.recent);
      first.dispose();

      final secondRelay = _FakeRelaySession()
        ..historyEvents = [
          event({'dms': 'recent'}, 100),
        ];
      final signed = _RecordingSignedEventRelay();
      final second = manager(secondRelay, signed);
      await second.initialize();

      expect(second.store.groups, {'channels': ChannelSortMode.recent});
      final submitted = await signed.submitted.future.timeout(
        const Duration(seconds: 1),
      );
      final payload =
          jsonDecode(crypto.decrypt(submitted.content)) as Map<String, dynamic>;
      expect(payload['groups'], {'channels': 'recent'});
      second.dispose();
    },
  );

  test('newer remote event cancels pending local whole-blob write', () async {
    final relay = _FakeRelaySession();
    final signed = _RecordingSignedEventRelay();
    final subject = manager(relay, signed);
    await subject.initialize();
    subject.setSortModeFor('channels', ChannelSortMode.recent);
    relay.emit(
      event({
        'dms': 'recent',
      }, DateTime.now().millisecondsSinceEpoch ~/ 1000 + 1),
    );
    await Future<void>.delayed(const Duration(milliseconds: 30));
    expect(subject.store.groups, {'dms': ChannelSortMode.recent});
    expect(signed.submitted.isCompleted, isFalse);
    subject.dispose();
  });

  test('same-second remote does not erase a pending local edit', () async {
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    ChannelSortStorage(prefs).writeSyncState(
      keys.public,
      'wss://relay.example',
      ChannelSortSyncState(updatedAt: now - 1, eventId: 'remote'),
    );
    final relay = _FakeRelaySession();
    final signed = _RecordingSignedEventRelay();
    final subject = manager(relay, signed);
    await subject.initialize();

    subject.setSortModeFor('channels', ChannelSortMode.recent);
    relay.emit(event({'dms': 'recent'}, now, id: 'a'));
    expect(subject.store.groups, {'channels': ChannelSortMode.recent});
    await signed.submitted.future.timeout(const Duration(seconds: 1));
    subject.dispose();
  });

  test('publishing does not advance the persisted remote cursor', () async {
    final storage = ChannelSortStorage(prefs);
    storage.writeSyncState(
      keys.public,
      'wss://relay.example',
      const ChannelSortSyncState(updatedAt: 100, eventId: 'remote'),
    );
    final signed = _RecordingSignedEventRelay();
    final subject = manager(_FakeRelaySession(), signed);
    await subject.initialize();

    subject.setSortModeFor('channels', ChannelSortMode.recent);
    await signed.submitted.future.timeout(const Duration(seconds: 1));
    await Future<void>.delayed(Duration.zero);

    final syncState = storage.readSyncState(keys.public, 'wss://relay.example');
    expect(syncState.updatedAt, 100);
    expect(syncState.eventId, 'remote');
    expect(syncState.hasPendingLocalChanges, isFalse);
    expect(syncState.pendingUpdatedAt, 0);
    subject.dispose();
  });

  test('legacy pending state resets its ambiguous remote cursor', () {
    final storage = ChannelSortStorage(prefs);
    // Simulate the pre-migration JSON, which had no pendingUpdatedAt field.
    prefs.setString(
      '${channelSortKey(keys.public, 'wss://relay.example')}:sync',
      jsonEncode({
        'updatedAt': 4102444800,
        'eventId': 'local-publication',
        'hasPendingLocalChanges': true,
      }),
    );

    final syncState = storage.readSyncState(keys.public, 'wss://relay.example');
    expect(syncState.updatedAt, 0);
    expect(syncState.eventId, isEmpty);
    expect(syncState.pendingUpdatedAt, 4102444800);
  });

  test('lower event ID wins when remote timestamps tie', () async {
    final relay = _FakeRelaySession()
      ..historyEvents = [
        event({'channels': 'recent'}, 200, id: 'f'),
      ];
    final subject = manager(relay, _RecordingSignedEventRelay());
    await subject.initialize();

    relay.emit(event({'dms': 'recent'}, 200, id: 'a'));
    expect(subject.store.groups, {'dms': ChannelSortMode.recent});

    relay.emit(event({'starred': 'recent'}, 200, id: 'z'));
    expect(subject.store.groups, {'dms': ChannelSortMode.recent});
    subject.dispose();
  });

  test('adopts a newer relay replacement after publishing', () async {
    final relay = _FakeRelaySession();
    final signed = _RecordingSignedEventRelay();
    final subject = manager(relay, signed);
    await subject.initialize();

    subject.setSortModeFor('channels', ChannelSortMode.recent);
    final submitted = await signed.submitted.future.timeout(
      const Duration(seconds: 1),
    );
    relay.emit(event({'dms': 'recent'}, submitted.createdAt! + 1, id: 'a'));
    expect(subject.store.groups, {'dms': ChannelSortMode.recent});
    subject.dispose();
  });

  test(
    'future-dated remote event is ignored and cannot wedge publishing',
    () async {
      final relay = _FakeRelaySession()
        ..historyEvents = [
          event({'channels': 'recent'}, 4102444800),
        ];
      final signed = _RecordingSignedEventRelay();
      final subject = manager(relay, signed);
      await subject.initialize();
      expect(subject.sortModeFor('channels'), ChannelSortMode.alpha);
      subject.setSortModeFor('dms', ChannelSortMode.recent);
      await signed.submitted.future.timeout(const Duration(seconds: 1));
      subject.dispose();
    },
  );

  test('retries failed startup and closes fetch-subscribe gap', () async {
    final relay = _FakeRelaySession()..fetchFailures = 1;
    final subject = manager(relay, _RecordingSignedEventRelay());
    await subject.initialize();
    relay.historyEvents = [
      event({'starred': 'recent'}, 300),
    ];
    await Future<void>.delayed(const Duration(milliseconds: 40));
    expect(subject.sortModeFor('starred'), ChannelSortMode.recent);
    expect(relay.fetchCount, greaterThanOrEqualTo(3));
    subject.dispose();
  });

  test('setSortModeFor prunes deleted custom section keys', () async {
    final subject = manager(_FakeRelaySession(), _RecordingSignedEventRelay());
    await subject.initialize();
    subject.setSortModeFor('section:dead', ChannelSortMode.recent);
    subject.setSortModeFor(
      'channels',
      ChannelSortMode.recent,
      liveSectionIds: ['live'],
    );
    expect(subject.store.groups.keys, ['channels']);
    subject.dispose();
  });
}

class _SubmittedEvent {
  final String content;
  final List<List<String>> tags;
  final int? createdAt;
  const _SubmittedEvent(this.content, this.tags, this.createdAt);
}

class _RecordingSignedEventRelay implements SignedEventRelay {
  final submitted = Completer<_SubmittedEvent>();
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
    onSigned?.call(
      const NostrEvent(
        id: 'signed-event',
        pubkey: '',
        createdAt: 0,
        kind: 0,
        tags: [],
        content: '',
        sig: '',
      ),
    );
    if (!submitted.isCompleted) {
      submitted.complete(_SubmittedEvent(content, tags, createdAt));
    }
    return const NostrEvent(
      id: 'ack',
      pubkey: '',
      createdAt: 0,
      kind: 0,
      tags: [],
      content: '',
      sig: '',
    );
  }
}

class _FakeRelaySession extends RelaySessionNotifier {
  List<NostrEvent> historyEvents = [];
  int fetchFailures = 0;
  int fetchCount = 0;
  void Function(NostrEvent)? _listener;

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    fetchCount++;
    if (fetchFailures > 0) {
      fetchFailures--;
      throw Exception('rate limited');
    }
    return historyEvents;
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    _listener = onEvent;
    return () => _listener = null;
  }

  void emit(NostrEvent event) => _listener?.call(event);
}
