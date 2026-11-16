import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/activity/feed_item.dart';
import 'package:buzz/features/activity/inbox_item.dart';
import 'package:buzz/features/activity/inbox_read_state.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:buzz/shared/read_state/read_state_format.dart';
import 'package:buzz/shared/read_state/read_state_manager.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/shared/read_state/read_state_storage.dart';
import 'package:buzz/shared/theme/community_theme_preference.dart';
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

  /// A reconnect: [device] reopens a lost subscription, and the replay's
  /// ready reads the snapshot.
  Future<void> reconnect(ReadStateManager device) async {
    await device.reinitializeRemote();
    relay.ready();
    await pumpEventQueue();
  }

  /// Publishes [from] and lets [to] read the relay, as a reconnect would.
  Future<void> sync(ReadStateManager from, ReadStateManager to) async {
    await from.flush();
    relay.ready();
    await pumpEventQueue();
  }

  Iterable<Map<String, int>> readStateBlobs() => [
    for (final event in relay.retained.values)
      if (hasValidReadStateTags(event))
        decodeReadStateBlob(crypto.decrypt(event.content))!.contexts
      else if (hasValidUnreadStateTags(event))
        decodeReadStateBlob(
          crypto.decrypt(event.content),
          version: 2,
        )!.contexts,
  ];

  /// [from]'s own unread record, as it last published it.
  Map<String, int> ownUnread(ReadStateManager from) {
    final journal = ReadStateStorage(prefs).readJournal(
      '${keys.public}:${Uri.encodeComponent(normalizeCommunityRelayUrl(from.relayUrl))}',
    )!;
    final record =
        relay.retained['$readStateUnreadDTagPrefix${journal['unreadSlot']}']!;
    return decodeReadStateBlob(
      crypto.decrypt(record.content),
      version: 2,
    )!.contexts;
  }

  /// The channel group in [from]'s own unread record.
  Map<String, int> ownGroup(ReadStateManager from) {
    final blob = ownUnread(from);
    return {
      for (final key in [
        channel,
        'ov_s:$channel',
        'ov_c:$channel',
        'ov_b:$channel',
      ])
        key: ?blob[key],
    };
  }

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
    relay.retained['read-state-unread-v2:peer'] = _readStateEvent(
      unread: true,
      keys: keys,
      crypto: crypto,
      clientId: 'peer',
      slot: 'peer',
      contexts: {
        message: 10,
        'ov_s:$message': clock - 20,
        'ov_c:$message': 0,
        'ov_b:$message': 10,
        thread: 10,
        'ov_s:$thread': clock - 20,
        'ov_c:$thread': 0,
        'ov_b:$thread': 10,
      },
      createdAt: clock - 10,
    );
    await reconnect(phone);
    phone.markContextRead(channel, clock - 5);
    await phone.flush();

    expect(phone.activeChannelOverrides, isEmpty);
    final own = relay.retained.entries
        .where(
          (e) =>
              e.key != 'read-state-unread-v2:peer' &&
              hasValidReadStateTags(e.value),
        )
        .map((e) => decodeReadStateBlob(crypto.decrypt(e.value.content))!)
        .single
        .contexts;
    expect(own.keys.where((key) => key.startsWith('ov_')), isEmpty);
  });

  test('an Inbox mark unread goes stale once a peer reads past it', () async {
    final phone = await device('phone');
    final desktop = await device('desktop');
    final row = buildInboxItems([
      FeedItem(
        id: 'b' * 64,
        kind: 9,
        pubkey: 'alice',
        content: 'hi',
        createdAt: clock - 100,
        channelId: channel,
        channelName: '',
        tags: const [],
        category: 'mention',
      ),
    ]).single;
    phone.markContextRead(channel, clock - 100);
    int? marker(String context) => phone.getEffectiveTimestamp(context);
    // Marked unread on the phone, at the frontier the row was read at.
    final overrides = {row.id: inboxItemReadAt(row, markerOf: marker)};
    bool done() => isInboxItemDone(
      row,
      markerOf: marker,
      localUnreadOverrides: overrides,
      localDoneSet: const {},
    );
    expect(done(), isFalse);

    desktop.markContextRead(channel, clock - 10, explicit: true);
    await sync(desktop, phone);

    expect(done(), isTrue);
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
      expect(await phone.setThreadFollow(channel, root, true), isTrue);
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
      expect(await phone.importLegacyFollows({key: true}), {key});
      expect(phone.threadFollows[key]!.follow, isFalse);

      final other = threadFollowKey(channel, 'e' * 64);
      expect(await phone.importLegacyFollows({other: true}), {other});
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
      await reconnect(phone);
      expect(phone.threadFollowsIncomplete, isTrue);
    });
  });

  group('sync seams', () {
    final key = threadFollowKey(channel, root);
    final slot = 'b' * 32;
    NostrEvent peerFollow(bool follow, int at, {String? content}) =>
        _followEvent(
          crypto: crypto,
          pubkey: keys.public,
          slot: slot,
          choices: {key: (follow: follow, at: at)},
          createdAt: at,
          content: content,
        );

    test('a stale follow delivery never replaces a newer head, in either '
        'order', () async {
      final phone = await device('phone');
      final older = peerFollow(true, clock - 20);
      final newer = peerFollow(false, clock - 10);
      relay
        ..deliver(newer)
        ..deliver(older);
      expect(phone.threadFollows[key]!.follow, isFalse);

      final desktop = await device('desktop');
      relay
        ..deliver(older)
        ..deliver(newer);
      expect(desktop.threadFollows[key]!.follow, isFalse);

      // A stale unreadable event does not hide its readable successor.
      relay.deliver(peerFollow(true, clock - 30, content: '{"v":2}'));
      expect(desktop.threadFollowsIncomplete, isFalse);
      expect(desktop.threadFollows[key]!.follow, isFalse);
    });

    test('restoring an older journal keeps a newer accepted choice', () async {
      final phone = await device('phone');
      phone.setThreadFollow(channel, root, true);
      await phone.flush();
      final backup = {
        for (final name in prefs.getKeys()) name: prefs.getString(name),
      };
      clock += 10;
      phone.setThreadFollow(channel, root, false);
      await phone.flush();
      phone.dispose(flushPending: false);
      for (final MapEntry(:key, :value) in backup.entries) {
        if (value != null) await prefs.setString(key, value);
      }

      final restored = await device('phone');
      await restored.flush();
      expect(restored.threadFollows[key], (follow: false, at: clock));
      expect(
        decodeThreadFollowsBlob(
          crypto.decrypt(relay.followEvents().single.content),
        )!.choices[key],
        (follow: false, at: clock),
      );
    });

    test('restoring a backup taken before a send keeps the newer accepted '
        'choice', () async {
      final phone = await device('phone');
      clock += 10;
      phone.setThreadFollow(channel, root, true);
      await phone.flush();
      clock += 10;
      phone.setThreadFollow(channel, root, false);
      await Future<void>.delayed(Duration.zero);
      final backup = {
        for (final name in prefs.getKeys()) name: prefs.getString(name),
      };
      await phone.flush();
      clock += 10;
      phone.setThreadFollow(channel, root, true);
      await phone.flush();
      phone.dispose(flushPending: false);
      for (final MapEntry(:key, :value) in backup.entries) {
        if (value != null) await prefs.setString(key, value);
      }

      final restored = await device('phone');
      restored.markContextRead(channel, clock - 1);
      await restored.flush();
      final accepted = (follow: true, at: clock);
      expect(
        decodeThreadFollowsBlob(
          crypto.decrypt(relay.followEvents().single.content),
        )!.choices[key],
        accepted,
      );
      expect((await device('reader')).threadFollows[key], accepted);
    });

    test('an unreadable head is decoded again after a restart', () async {
      final phone = await device('phone');
      phone.setThreadFollow(channel, root, true);
      await phone.flush();
      final own = relay.followEvents().single;
      final ownSlot = threadFollowsSlotOf(own)!;
      NostrEvent unreadable(String slot) => NostrEvent(
        id: 'unreadable-$slot',
        pubkey: keys.public,
        createdAt: clock + 5,
        kind: EventKind.readState,
        tags: [
          ['d', '$threadFollowsDTagPrefix$slot'],
          ['t', threadFollowsTag],
        ],
        content: crypto.encrypt('{"v":2}'),
        sig: 'sig',
      );
      relay.retained['$threadFollowsDTagPrefix$ownSlot'] = unreadable(ownSlot);
      relay.retained['${threadFollowsDTagPrefix}peer'] = unreadable(slot);
      await reconnect(phone);
      expect(phone.threadFollowsIncomplete, isTrue);
      phone.dispose(flushPending: false);

      final restarted = await device('phone');
      expect(restarted.threadFollowsIncomplete, isTrue);
      clock += 10;
      restarted.setThreadFollow(channel, 'c' * 64, true);
      await restarted.flush();
      expect(
        relay.retained['$threadFollowsDTagPrefix$ownSlot']!.id,
        'unreadable-$ownSlot',
      );
    });

    test('an unreadable peer head alone keeps follows incomplete after a '
        'restart', () async {
      final phone = await device('phone');
      relay.retained['${threadFollowsDTagPrefix}peer'] = NostrEvent(
        id: 'unreadable-peer',
        pubkey: keys.public,
        createdAt: clock,
        kind: EventKind.readState,
        tags: [
          ['d', '$threadFollowsDTagPrefix$slot'],
          ['t', threadFollowsTag],
        ],
        content: crypto.encrypt('{"v":2}'),
        sig: 'sig',
      );
      await reconnect(phone);
      expect(phone.threadFollowsIncomplete, isTrue);
      phone.dispose(flushPending: false);

      relay.offline = true;
      final restarted = await device('phone');
      expect(restarted.threadFollowsIncomplete, isTrue);
      relay.offline = false;
      await reconnect(restarted);
      expect(restarted.threadFollowsIncomplete, isTrue);
    });

    test('a choice made before a retry is sent after it', () async {
      final phone = await device('phone');
      relay.failNext = true;
      phone.setThreadFollow(channel, root, true);
      await phone.flush();
      clock++;
      phone.setThreadFollow(channel, root, false);
      await phone.flush();

      expect(
        decodeThreadFollowsBlob(
          crypto.decrypt(relay.followEvents().single.content),
        )!.choices[key],
        (follow: false, at: clock),
      );
    });

    test('peer follow choices survive an offline restart', () async {
      final desktop = await device('desktop');
      final other = threadFollowKey(channel, 'c' * 64);
      desktop.setThreadFollow(channel, root, true);
      desktop.setThreadFollow(channel, 'c' * 64, false);
      final phone = await device('phone');
      await sync(desktop, phone);
      phone.dispose(flushPending: false);

      relay.offline = true;
      final restarted = await device('phone');
      expect(restarted.threadFollows[key]!.follow, isTrue);
      expect(restarted.threadFollows[other]!.follow, isFalse);
    });

    test('an older build replays the journal without a follow event', () async {
      final phone = await device('phone');
      relay.failNext = true;
      phone.setThreadFollow(channel, root, true);
      await phone.flush();
      final journal = [
        for (final name in prefs.getKeys())
          if (prefs.getString(name) case final raw?
              when raw.contains('"followPending"'))
            jsonDecode(raw) as Map<String, dynamic>,
      ].single;
      expect(journal['followPending'] as Map, isNotEmpty);
      // The older build decodes every `pending` entry as read state.
      for (final event in (journal['pending'] as Map).values) {
        final content = NostrEvent.fromJson(
          Map<String, dynamic>.from(event as Map),
        ).content;
        expect(decodeReadStateBlob(crypto.decrypt(content)), isNotNull);
      }
    });

    test(
      'an overfull merged primary still publishes reads and follows',
      () async {
        for (var peer = 0; peer < 3; peer++) {
          relay.retained['read-state-unread-v2:peer$peer'] = _readStateEvent(
            unread: true,
            keys: keys,
            crypto: crypto,
            clientId: 'peer$peer',
            slot: 'peer$peer',
            contexts: {
              for (var index = 0; index < 160; index++)
                for (final id in ['$peer-$index'.padLeft(36, '0')]) ...{
                  id: clock - 100,
                  'ov_s:$id': clock - 90,
                  'ov_c:$id': 0,
                  'ov_b:$id': clock - 100,
                },
            },
            createdAt: clock - 50,
          );
        }
        final phone = await device('phone');
        expect(phone.activeChannelOverrides, hasLength(480));
        expect(phone.markChannelUnread(channel), isFalse);
        phone.markContextRead(channel, clock - 1);
        phone.setThreadFollow(channel, root, true);
        await phone.flush();

        expect(phone.syncError, isNull);
        final desktop = await device('desktop');
        expect(desktop.getEffectiveTimestamp(channel), clock - 1);
        expect(desktop.threadFollows[key]!.follow, isTrue);
      },
    );

    Future<void> overfill() async {
      for (var peer = 0; peer < 3; peer++) {
        relay.retained['read-state-unread-v2:peer$peer'] = _readStateEvent(
          unread: true,
          keys: keys,
          crypto: crypto,
          clientId: 'peer$peer',
          slot: 'peer$peer',
          contexts: {
            for (var index = 0; index < 160; index++)
              for (final id in ['$peer-$index'.padLeft(36, '0')]) ...{
                id: clock - 100,
                'ov_s:$id': clock - 90,
                'ov_c:$id': 0,
                'ov_b:$id': clock - 100,
              },
          },
          createdAt: clock - 50,
        );
      }
    }

    test(
      'an overfull fallback publishes a dead own group as a tombstone',
      () async {
        final phone = await device('phone');
        phone.markContextRead(channel, clock - 100);
        final marked = clock;
        expect(phone.markChannelUnread(channel), isTrue);
        await phone.flush();
        await overfill();
        await reconnect(phone);

        phone.markContextRead(channel, clock - 1);
        await phone.flush();
        final own = ownUnread(phone);
        expect(own[channel], clock - 1);
        expect(own.containsKey('ov_s:$channel'), isFalse);
        expect(own['ov_c:$channel'], marked);
      },
    );

    test('a clear the primary cannot carry, or an exhausted counter, '
        'fails visibly', () async {
      final phone = await device('phone');
      phone.markContextRead(channel, clock - 100);
      expect(phone.markChannelUnread(channel), isTrue);
      await phone.flush();
      await overfill();
      await reconnect(phone);
      expect(phone.clearChannelOverride(channel), isFalse);
      expect(phone.syncError, isNotNull);
      expect(phone.activeChannelOverrides, contains(channel));

      relay.retained.clear();
      final other = 'f' * 36;
      // A peer stamped 10 minutes ahead: within the deferral bound, but no
      // later stamp can be allocated within a minute of this clock.
      relay.retained['read-state-unread-v2:max'] = _readStateEvent(
        unread: true,
        keys: keys,
        crypto: crypto,
        clientId: 'max',
        slot: 'max',
        contexts: {
          other: clock - 100,
          'ov_s:$other': clock + 600,
          'ov_c:$other': 0,
          'ov_b:$other': clock - 100,
        },
        createdAt: clock - 50,
      );
      final desktop = await device('desktop');
      expect(desktop.clearChannelOverride(other), isFalse);
      expect(desktop.syncError, isNotNull);
      expect(desktop.activeChannelOverrides, {other});
    });

    test('a failed clear stays reported until a clear succeeds', () async {
      final phone = await device('phone');
      phone.markContextRead(channel, clock - 100);
      expect(phone.markChannelUnread(channel), isTrue);
      await phone.flush();
      await overfill();
      await reconnect(phone);
      expect(phone.clearChannelOverride(channel), isFalse);

      // Unrelated syncing, with nothing else to send, does not hide it.
      await phone.flush();
      await reconnect(phone);
      await phone.flush();
      expect(phone.syncError, isNotNull);
      expect(phone.activeChannelOverrides, contains(channel));
    });

    test('a live peer group schedules a canonical publish', () async {
      final phone = await device('phone');
      final desktop = await device('desktop');
      phone.markContextRead(channel, clock - 100);
      expect(phone.markChannelUnread(channel), isTrue);
      await phone.flush();
      expect(desktop.activeChannelOverrides, {channel});

      // The debounced publish carries the merged group in the desktop's
      // own primary.
      await Future<void>.delayed(const Duration(seconds: 6));
      expect(
        readStateBlobs().where((blob) => blob['ov_s:$channel'] != null),
        hasLength(2),
      );
    });

    for (final (name, frontier) in [
      ('never read', null),
      ('read over 30 days ago', 1800000000 - 40 * 24 * 3600),
    ]) {
      test('mark unread on a channel $name publishes a live group after the '
          'debounce', () async {
        final phone = await device('phone');
        if (frontier != null) phone.markContextRead(channel, frontier);
        expect(phone.markChannelUnread(channel), isTrue);
        clock += 6;
        await Future<void>.delayed(const Duration(seconds: 6));

        final base = frontier ?? 0;
        expect(ownGroup(phone), {
          channel: base,
          'ov_s:$channel': clock - 6,
          'ov_c:$channel': 0,
          'ov_b:$channel': base,
        });
        final desktop = await device('desktop');
        expect(desktop.activeChannelOverrides, {channel});
        // The floor still reads the channel's old messages.
        final state = ReadStateState(
          isReady: true,
          pubkey: keys.public,
          contexts: phone.effectiveContexts,
          version: 1,
          retentionFloor: readStateFloor(clock),
        );
        expect(state.effectiveTimestamp(channel), readStateFloor(clock));
      });
    }

    test('a peer group with a zero frontier survives receive, journal and an '
        'offline restart', () async {
      relay.retained['read-state-unread-v2:peer'] = _readStateEvent(
        unread: true,
        keys: keys,
        crypto: crypto,
        clientId: 'peer',
        slot: 'peer',
        contexts: {
          channel: 0,
          'ov_s:$channel': clock - 20,
          'ov_c:$channel': 0,
          'ov_b:$channel': 0,
        },
        createdAt: clock - 10,
      );
      final phone = await device('phone');
      expect(phone.activeChannelOverrides, {channel});
      await phone.flush();
      expect(ownGroup(phone), {
        channel: 0,
        'ov_s:$channel': clock - 20,
        'ov_c:$channel': 0,
        'ov_b:$channel': 0,
      });

      relay.offline = true;
      final restarted = await device('phone');
      expect(restarted.activeChannelOverrides, {channel});
    });

    test('a real advance, or a same-frontier clear, still ends a mark on a '
        'never-read channel', () async {
      final phone = await device('phone');
      expect(phone.markChannelUnread(channel), isTrue);
      phone.markContextRead(channel, clock - 1);
      expect(phone.activeChannelOverrides, isEmpty);

      final other = '${channel}b';
      expect(phone.markChannelUnread(other), isTrue);
      expect(phone.clearChannelOverride(other), isTrue);
      expect(phone.activeChannelOverrides, isEmpty);
      await phone.flush();
      final blob = ownUnread(phone);
      expect(blob[other], 0);
      // The clear takes a fresh stamp, after the mark's in the same second.
      expect(blob['ov_c:$other'], clock + 1);
      expect(blob.containsKey('ov_s:$other'), isFalse);
    });

    test('the overfull fallback keeps live groups with a zero or old '
        'frontier', () async {
      final phone = await device('phone');
      final old = clock - 40 * 24 * 3600;
      final stale = '${channel}b';
      phone.markContextRead(stale, old);
      expect(phone.markChannelUnread(channel), isTrue);
      expect(phone.markChannelUnread(stale), isTrue);
      await phone.flush();
      await overfill();
      await reconnect(phone);

      clock += 3600;
      phone.markContextRead('${channel}c', clock - 1);
      await phone.flush();
      expect(phone.syncError, isNull);
      final own = ownUnread(phone);
      expect(
        {
          for (final key in [
            channel,
            'ov_s:$channel',
            'ov_b:$channel',
            stale,
            'ov_s:$stale',
            'ov_b:$stale',
          ])
            key: own[key],
        },
        {
          channel: 0,
          'ov_s:$channel': clock - 3600,
          'ov_b:$channel': 0,
          stale: old,
          'ov_s:$stale': clock - 3600,
          'ov_b:$stale': old,
        },
      );
    });
  });

  group('expiring unread (version 2)', () {
    const lifetime = 30 * 24 * 3600;

    test('a mark expires 30 days after it was made, even unread', () async {
      final phone = await device('phone');
      phone.markContextRead(channel, clock - 100);
      final marked = clock;
      expect(phone.markChannelUnread(channel), isTrue);
      await phone.flush();

      clock = marked + lifetime;
      expect(phone.activeChannelOverrides, {channel});
      clock += 1;
      expect(phone.activeChannelOverrides, isEmpty);
      await phone.flush();
      expect(ownGroup(phone), {channel: marked - 100, 'ov_c:$channel': marked});
      final desktop = await device('desktop');
      expect(desktop.activeChannelOverrides, isEmpty);
    });

    test('a dead group is discarded only past the lifetime and guard, and its '
        'replay stays dead', () async {
      final phone = await device('phone');
      phone.markContextRead(channel, clock - 100);
      final marked = clock;
      expect(phone.markChannelUnread(channel), isTrue);
      await phone.flush();
      final replay = relay.retained.entries
          .firstWhere((e) => e.key.startsWith(readStateUnreadDTagPrefix))
          .value;

      clock = marked + lifetime + 600;
      await phone.flush();
      expect(ownUnread(phone), {
        channel: marked - 100,
        'ov_c:$channel': marked,
      });
      clock += 1;
      await phone.flush();
      expect(ownUnread(phone), isEmpty);

      // The old live group, replayed by a peer, is too old to merge.
      relay.retained['${readStateUnreadDTagPrefix}peer'] = replay;
      await reconnect(phone);
      expect(phone.activeChannelOverrides, isEmpty);
      relay.offline = true;
      final restarted = await device('phone');
      expect(restarted.activeChannelOverrides, isEmpty);
      // A new mark is newer than the discarded ceiling.
      relay.offline = false;
      await reconnect(restarted);
      expect(restarted.markChannelUnread(channel), isTrue);
      expect(restarted.activeChannelOverrides, {channel});
    });

    test('a clock rollback does not revive an expired mark', () async {
      final phone = await device('phone');
      phone.markContextRead(channel, clock - 100);
      final marked = clock;
      expect(phone.markChannelUnread(channel), isTrue);
      await phone.flush();
      clock = marked + lifetime + 1;
      expect(phone.activeChannelOverrides, isEmpty);
      await phone.flush();

      clock = marked;
      expect(phone.activeChannelOverrides, isEmpty);
      relay.offline = true;
      final restarted = await device('phone');
      expect(restarted.activeChannelOverrides, isEmpty);
    });

    test('a group stamped too far ahead is deferred, and the view is '
        'incomplete until the clock reaches it', () async {
      final other = '${channel}b';
      relay.retained['${readStateUnreadDTagPrefix}peer'] = _readStateEvent(
        unread: true,
        keys: keys,
        crypto: crypto,
        clientId: 'peer',
        slot: 'peer',
        contexts: {
          other: clock - 100,
          'ov_s:$other': clock + 700,
          'ov_c:$other': 0,
          'ov_b:$other': clock - 100,
        },
        createdAt: clock - 10,
      );
      final phone = await device('phone');
      expect(phone.activeChannelOverrides, isEmpty);
      expect(phone.markChannelUnread(channel), isFalse);

      clock += 100;
      expect(phone.markChannelUnread(channel), isTrue);
      expect(phone.activeChannelOverrides, {channel, other});
    });

    test('a v1 override group is not imported', () async {
      relay.retained['read-state:old'] = _readStateEvent(
        keys: keys,
        crypto: crypto,
        clientId: 'old',
        slot: 'old',
        contexts: {
          channel: clock - 100,
          'ov_s:$channel': clock - 50,
          'ov_c:$channel': 0,
          'ov_b:$channel': clock - 100,
        },
        createdAt: clock - 10,
      );
      final phone = await device('phone');
      expect(phone.activeChannelOverrides, isEmpty);
      expect(phone.getEffectiveTimestamp(channel), clock - 100);
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

    test('measures each slot with its own unknown fields', () {
      final first = packThreadFollowKeys(entries(200), clientId: 'c');
      final all = entries(260);
      final extras = [
        {'future': 'x' * 10000},
        {'future': ''},
      ];
      final placed = packThreadFollowKeys(
        all,
        clientId: 'c',
        previous: [first.first, {}],
        extras: extras,
      );
      for (var index = 0; index < placed.length; index++) {
        final plaintext = ThreadFollowsBlob(
          clientId: 'c',
          entries: {for (final key in placed[index]) key: all[key]!},
          extra: index < extras.length ? extras[index] : const {},
        ).encode();
        expect(
          utf8.encode(plaintext).length,
          lessThanOrEqualTo(readStatePlaintextBytes),
        );
      }
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
  // The version-2 unread record instead of read positions.
  bool unread = false,
}) => NostrEvent(
  id: 'event-$clientId-$createdAt',
  pubkey: keys.public,
  createdAt: createdAt,
  kind: EventKind.readState,
  tags: [
    ['d', '${unread ? readStateUnreadDTagPrefix : readStateDTagPrefix}$slot'],
    ['t', unread ? readStateUnreadTag : 'read-state'],
  ],
  content: crypto.encrypt(
    jsonEncode(
      ReadStateBlob(
        clientId: clientId,
        contexts: contexts,
        version: unread ? 2 : 1,
      ).toJson(),
    ),
  ),
  sig: 'sig',
);

NostrEvent _followEvent({
  required ReadStateCrypto crypto,
  required String pubkey,
  required String slot,
  required Map<String, ThreadFollowChoice> choices,
  required int createdAt,
  String clientId = 'peer',
  String? content,
}) => NostrEvent(
  id: 'follow-$slot-$createdAt',
  pubkey: pubkey,
  createdAt: createdAt,
  kind: EventKind.readState,
  tags: [
    ['d', '$threadFollowsDTagPrefix$slot'],
    ['t', threadFollowsTag],
  ],
  content: crypto.encrypt(
    content ??
        ThreadFollowsBlob(
          clientId: clientId,
          entries: {
            for (final MapEntry(:key, :value) in choices.entries)
              key: {'follow': value.follow, 'at': value.at},
          },
        ).encode(),
  ),
  sig: 'sig',
);

/// A relay that keeps the newest event per coordinate and serves them all as
/// the read-state snapshot.
class _MemoryRelay extends RelaySessionNotifier {
  final retained = <String, NostrEvent>{};
  final _live = <void Function(NostrEvent)>[];
  bool failNext = false;
  bool offline = false;

  /// Delivers [event] to live subscribers without retaining it.
  void deliver(NostrEvent event) {
    for (final onEvent in List.of(_live)) {
      onEvent(event);
    }
  }

  List<NostrEvent> followEvents() => [
    for (final event in retained.values)
      if (threadFollowsSlotOf(event) != null) event,
  ];

  @override
  Future<List<NostrEvent>> fetchReadStateSnapshot({
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (offline) throw TimeoutException('offline');
    return retained.values.toList();
  }

  final List<void Function(RelaySubscriptionStatus)> _statusListeners = [];

  /// Reports every live subscription ready again, as a reconnect's replay
  /// does.
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
    if (offline) throw TimeoutException('offline');
    _live.add(onEvent);
    return () => _live.remove(onEvent);
  }

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
    final head = retained[d];
    // NIP-01 replaceable: a newer created_at wins, then the lower id.
    if (head == null ||
        event.createdAt > head.createdAt ||
        event.createdAt == head.createdAt && event.id.compareTo(head.id) < 0) {
      retained[d] = event;
    }
    deliver(event);
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
