import 'dart:async';
import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:buzz/shared/read_state/read_state_format.dart';
import 'package:buzz/shared/read_state/read_state_manager.dart';
import 'package:buzz/shared/read_state/thread_follows_format.dart';
import 'package:buzz/shared/relay/relay.dart';

/// Two installations of one identity against one in-memory relay: the NIP-RS
/// Buzz profile's synced mark read, whole-channel mark unread and thread
/// follows. Each device gets its own storage scope through its own relay URL.
void main() {
  const channel = 'chan-1';
  final root = 'a' * 64;
  late nostr.Keys keys;
  late ReadStateCrypto crypto;
  late _MemoryRelay relay;
  late SharedPreferences prefs;
  var clock = 1800000000;

  setUp(() async {
    SharedPreferences.setMockInitialValues({});
    prefs = await SharedPreferences.getInstance();
    keys = nostr.Keys.generate();
    crypto = ReadStateCrypto.tryCreate(nsec: keys.nsec, pubkey: keys.public)!;
    relay = _MemoryRelay();
    clock = 1800000000;
  });

  Future<ReadStateManager> device(String name) async {
    final manager = ReadStateManager(
      pubkey: keys.public,
      relayUrl: 'https://$name.example',
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

  /// Publishes [from] and lets [to] read the relay, as a reconnect would.
  Future<void> sync(ReadStateManager from, ReadStateManager to) async {
    await from.flush();
    await to.reinitializeRemote();
  }

  Iterable<Map<String, int>> readStateBlobs() => [
    for (final event in relay.retained.values)
      if (hasValidReadStateTags(event))
        decodeReadStateBlob(crypto.decrypt(event.content))!.contexts,
  ];

  test('whole-channel mark unread round-trips between devices', () async {
    final phone = await device('phone');
    final desktop = await device('desktop');
    phone.markContextRead(channel, clock - 100);
    await sync(phone, desktop);

    expect(phone.markChannelUnread(channel), isTrue);
    await sync(phone, desktop);
    expect(desktop.activeChannelOverrides, {channel});

    // A clear that moves no frontier still syncs, through the counter.
    desktop.clearChannelOverride(channel);
    await sync(desktop, phone);
    expect(phone.activeChannelOverrides, isEmpty);
    expect(phone.getEffectiveTimestamp(channel), clock - 100);
  });

  test('an explicit channel read clears the synced manual unread', () async {
    final phone = await device('phone');
    final desktop = await device('desktop');
    phone.markContextRead(channel, clock - 100);
    expect(phone.markChannelUnread(channel), isTrue);
    await sync(phone, desktop);

    desktop.markContextRead(channel, clock - 100, explicit: true);
    await sync(desktop, phone);

    expect(phone.activeChannelOverrides, isEmpty);
  });

  test('a peer read that never saw the mark unread deactivates it', () async {
    final phone = await device('phone');
    final desktop = await device('desktop');
    phone.markContextRead(channel, clock - 100);
    expect(phone.markChannelUnread(channel), isTrue);

    // The desktop, not yet merged, catches up past the baseline.
    desktop.markContextRead(channel, clock - 50);
    await sync(desktop, phone);
    await sync(phone, desktop);

    // Accepted limitation: the unread is lost and nothing restores it.
    expect(phone.activeChannelOverrides, isEmpty);
    expect(desktop.activeChannelOverrides, isEmpty);
  });

  test('a stray message or thread group from a peer is ignored and not '
      'carried', () async {
    final phone = await device('phone');
    final message = msgContextKey('b' * 64);
    final thread = threadContextKey(root);
    relay.retained['read-state:peer'] = _readStateEvent(
      keys: keys,
      crypto: crypto,
      clientId: 'peer',
      slot: 'peer',
      contexts: {
        message: 10,
        'ov_s:$message': 1,
        'ov_c:$message': 0,
        'ov_b:$message': 10,
        thread: 10,
        'ov_s:$thread': 1,
        'ov_c:$thread': 0,
        'ov_b:$thread': 10,
      },
      createdAt: clock - 10,
    );
    await phone.reinitializeRemote();
    phone.markContextRead(channel, clock - 5);
    await phone.flush();

    expect(phone.activeChannelOverrides, isEmpty);
    final own = relay.retained.entries
        .where(
          (e) => e.key != 'read-state:peer' && hasValidReadStateTags(e.value),
        )
        .map((e) => decodeReadStateBlob(crypto.decrypt(e.value.content))!)
        .single
        .contexts;
    expect(own.keys.where((key) => key.startsWith('ov_')), isEmpty);
  });

  test('a first explicit message read publishes only its frontier', () async {
    final phone = await device('phone');
    final message = msgContextKey('c' * 64);
    phone.markContextRead(message, clock - 20, explicit: true);
    // Passive reading of another message stays on this device.
    phone.markContextRead(msgContextKey('d' * 64), clock - 20);
    await phone.flush();

    final published = readStateBlobs().single;
    expect(published, {message: clock - 20});
  });

  test('channel mark unread fails visibly when the primary is full', () async {
    final phone = await device('phone');
    var failed = false;
    for (var index = 0; index < 1000 && !failed; index++) {
      failed = !phone.markChannelUnread(index.toString().padLeft(36, '0'));
    }
    expect(failed, isTrue);
    // Ordinary reading still works at the ceiling.
    phone.markContextRead(channel, clock - 1);
    expect(phone.getEffectiveTimestamp(channel), clock - 1);
  });

  group('thread follows', () {
    test('a follow on one device reaches the other', () async {
      final phone = await device('phone');
      final desktop = await device('desktop');
      expect(phone.setThreadFollow(channel, root, true), isTrue);
      await sync(phone, desktop);

      expect(desktop.threadFollows[threadFollowKey(channel, root)], (
        follow: true,
        at: clock,
      ));
      expect(desktop.threadFollowsIncomplete, isFalse);
    });

    test('a newer unfollow on another device wins, and a same-second tie '
        'resolves to unfollow', () async {
      final phone = await device('phone');
      final desktop = await device('desktop');
      final key = threadFollowKey(channel, root);
      phone.setThreadFollow(channel, root, true);
      clock++;
      desktop.setThreadFollow(channel, root, false);
      await sync(phone, desktop);
      await sync(desktop, phone);
      expect(phone.threadFollows[key]!.follow, isFalse);

      // Same second, opposite choices.
      clock++;
      phone.setThreadFollow(channel, root, true);
      desktop.setThreadFollow(channel, root, false);
      await sync(phone, desktop);
      await sync(desktop, phone);
      expect(phone.threadFollows[key]!.follow, isFalse);
      expect(desktop.threadFollows[key]!.follow, isFalse);
    });

    test('a legacy import at at = 0 never beats a real choice', () async {
      final phone = await device('phone');
      final desktop = await device('desktop');
      final key = threadFollowKey(channel, root);
      desktop.setThreadFollow(channel, root, false);
      await sync(desktop, phone);

      // Already synced: settled without writing.
      expect(phone.importLegacyFollows({key: true}), {key});
      expect(phone.threadFollows[key]!.follow, isFalse);

      final other = threadFollowKey(channel, 'e' * 64);
      expect(phone.importLegacyFollows({other: true}), {other});
      expect(phone.threadFollows[other], (follow: true, at: 0));
    });

    test('a backward clock still publishes a newer event', () async {
      final phone = await device('phone');
      phone.setThreadFollow(channel, root, true);
      await phone.flush();
      final first = relay.followEvents().single.createdAt;

      clock -= 3600;
      phone.setThreadFollow(channel, root, false);
      await phone.flush();

      final second = relay.followEvents().single;
      expect(second.createdAt, greaterThan(first));
      expect(
        decodeThreadFollowsBlob(
          crypto.decrypt(second.content),
        )!.choices[threadFollowKey(channel, root)],
        (follow: false, at: clock),
      );
    });

    test('a failed publication is retried after restart', () async {
      final phone = await device('phone');
      relay.failNext = true;
      phone.setThreadFollow(channel, root, true);
      await phone.flush();
      expect(phone.syncError, isNotNull);
      phone.dispose(flushPending: false);

      final restarted = await device('phone');
      await restarted.flush();
      expect(restarted.syncError, isNull);
      expect(
        decodeThreadFollowsBlob(
          crypto.decrypt(relay.followEvents().single.content),
        )!.choices.keys,
        [threadFollowKey(channel, root)],
      );
    });

    test('an unreadable follow coordinate leaves state incomplete', () async {
      final phone = await device('phone');
      relay.retained['thread-follows:${'f' * 32}'] = NostrEvent(
        id: 'x',
        pubkey: keys.public,
        createdAt: clock,
        kind: EventKind.readState,
        tags: [
          ['d', 'thread-follows:${'f' * 32}'],
          ['t', threadFollowsTag],
        ],
        content: crypto.encrypt('{"v":2}'),
        sig: 'sig',
      );
      await phone.reinitializeRemote();
      expect(phone.threadFollowsIncomplete, isTrue);
    });
  });

  group('follow slot packing', () {
    Map<String, Map<String, Object?>> entries(int count) => {
      for (var index = 0; index < count; index++)
        threadFollowKey(
          '00000000-0000-0000-0000-000000000000',
          index.toRadixString(16).padLeft(64, '0'),
        ): {
          'follow': false,
          'at': 1794612408,
        },
    };

    test('spills to more slots without moving placed keys', () {
      final first = packThreadFollowKeys(entries(200), clientId: 'c');
      expect(first, hasLength(1));
      final second = packThreadFollowKeys(
        entries(700),
        clientId: 'c',
        previous: first,
      );
      expect(second.length, greaterThan(1));
      expect(second.first.containsAll(first.first), isTrue);
    });

    test('refuses past the slot ceiling', () {
      expect(
        () => packThreadFollowKeys(
          entries(threadFollowsMaxSlots * 320),
          clientId: 'c',
        ),
        throwsStateError,
      );
    });
  });
}

NostrEvent _readStateEvent({
  required nostr.Keys keys,
  required ReadStateCrypto crypto,
  required String clientId,
  required String slot,
  required Map<String, int> contexts,
  required int createdAt,
}) => NostrEvent(
  id: 'event-$clientId-$createdAt',
  pubkey: keys.public,
  createdAt: createdAt,
  kind: EventKind.readState,
  tags: [
    ['d', '$readStateDTagPrefix$slot'],
    ['t', 'read-state'],
  ],
  content: crypto.encrypt(
    jsonEncode(ReadStateBlob(clientId: clientId, contexts: contexts).toJson()),
  ),
  sig: 'sig',
);

/// A relay that keeps the newest event per coordinate and serves them all as
/// the read-state snapshot.
class _MemoryRelay extends RelaySessionNotifier {
  final retained = <String, NostrEvent>{};
  bool failNext = false;

  List<NostrEvent> followEvents() => [
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
  }) async => () {};

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (failNext) {
      failNext = false;
      throw TimeoutException('relay timeout');
    }
    final d = event.tags.firstWhere((tag) => tag.first == 'd')[1];
    retained[d] = event;
    return event;
  }
}
