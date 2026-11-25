import 'dart:async';
import 'dart:convert';
import 'package:buzz/features/channels/channel_filter/channel_filter_manager.dart';
import 'package:buzz/features/channels/channel_filter/channel_filter_store.dart';
import 'package:buzz/features/channels/channel_sort/channel_sort_manager.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  test('drops fractional JSON types from preview counts', () {
    final store = ChannelFilterStore.parse({
      'version': 1,
      'previewCounts': {'channels': 10.0, 'dms': 15},
    });
    expect(store.countFor('channels'), 5);
    expect(store.countFor('dms'), 15);
  });
  test(
    'section count survives Active and is independent of other sections',
    () {
      var store = ChannelFilterStore.empty();
      expect(store.limitFor('channels'), 5);
      for (final count in sectionCounts) {
        store = store.edit(
          'channels',
          ChannelFilterMode.unreads,
          previewCount: count,
        );
        store = ChannelFilterStore.parse(store.toJson());
        expect(store.limitFor('channels'), count == 0 ? isNull : count);
        store = store.edit(
          'channels',
          ChannelFilterMode.active,
          windowSeconds: 86400,
        );
        expect(store.limitFor('channels'), isNull);
        store = store.edit('channels', ChannelFilterMode.all);
        expect(store.countFor('channels'), count);
        expect(store.limitFor('dms'), 5);
      }
    },
  );

  test(
    'metadata wins and edits preserve reset tombstones, unknown fields and windows',
    () {
      final source = {
        'version': 1,
        'groups': {'channels': 'active'},
        'future': {'keep': true},
        'activeWindowSeconds': {'channels': 604800, 'forums': 86400},
        'meta': {
          'v': 1,
          'g': {
            'channels': [12, '0000000000000000', null],
            'forums': [13, '0000000000000000', 'active'],
          },
        },
      };
      final store = ChannelFilterStore.parse(source);
      expect(store.modeFor('channels'), ChannelFilterMode.all);
      final next = store.edit('dms', ChannelFilterMode.unreads).toJson();
      expect(next['future'], source['future']);
      expect(next['activeWindowSeconds'], source['activeWindowSeconds']);
      expect(
        next['meta']['g']['channels'],
        ((source['meta'] as Map)['g'] as Map)['channels'],
      );
      expect(next['groups'], {'forums': 'active', 'dms': 'unreads'});
      expect(store.modeFor('dms'), ChannelFilterMode.all);
    },
  );
  test(
    'All writes a tombstone, preserves duration; section deletion removes duration',
    () {
      final active = ChannelFilterStore.empty().edit(
        'section:abc',
        ChannelFilterMode.active,
        windowSeconds: 86400,
      );
      final reset = active.edit('section:abc', ChannelFilterMode.all);
      expect(reset.windowFor('section:abc'), 86400);
      expect(reset.toJson()['meta']['g']['section:abc'][2], isNull);
      expect(
        reset
            .edit('section:abc', ChannelFilterMode.all, removeWindow: true)
            .toJson()['activeWindowSeconds'],
        isEmpty,
      );
    },
  );
  test(
    'salvages invalid durations but rejects unsupported metadata and oversized records',
    () {
      for (final invalid in [0, 86399, 86401, 366 * 86400, '86400']) {
        expect(
          ChannelFilterStore.parse({
            'version': 1,
            'groups': {},
            'activeWindowSeconds': {'channels': invalid},
          }).windowFor('channels'),
          defaultActiveWindowSeconds,
        );
      }
      expect(
        () => ChannelFilterStore.parse({
          'version': 1,
          'groups': {},
          'meta': {'v': 2, 'g': {}},
        }),
        throwsFormatException,
      );
      expect(
        () => ChannelFilterStore.parse({
          'version': 1,
          'groups': {},
          'future': 'x' * (128 * 1024),
        }),
        throwsFormatException,
      );
    },
  );
  test(
    'filter predicate covers selection, unread, unknown/empty activity and exact expiry',
    () {
      for (final mode in ChannelFilterMode.values) {
        for (final selected in [false, true]) {
          for (final unread in [false, true]) {
            for (final known in [false, true]) {
              for (final at in <int?>[null, 0, 1, 2]) {
                final result = channelMatchesFilter(
                  mode: mode,
                  selected: selected,
                  unread: unread,
                  activityKnown: known,
                  activitySeconds: at,
                  nowSeconds: 11,
                  windowSeconds: 10,
                );
                final expected =
                    selected ||
                    mode == ChannelFilterMode.all ||
                    (mode == ChannelFilterMode.unreads && unread) ||
                    (mode == ChannelFilterMode.active &&
                        (at == null ? !known : at > 1));
                expect(
                  result,
                  expected,
                  reason: '$mode $selected $unread $known $at',
                );
              }
            }
          }
        }
      }
    },
  );

  late nostr.Keys keys;
  late ChannelSortCrypto crypto;
  late List<NostrEvent> history;
  var failRead = false;
  var failPublish = false;
  var retainPublish = true;
  var live = true;
  var publishes = 0;
  late List<ChannelFilterStore> cached;
  setUp(() {
    keys = nostr.Keys.generate();
    crypto = ChannelSortCrypto(keys.nsec, keys.public);
    history = [];
    cached = [];
    failRead = false;
    failPublish = false;
    retainPublish = true;
    publishes = 0;
    live = true;
  });
  NostrEvent signed(String content, int at) => NostrEvent.fromJson(
    nostr.Event.from(
      kind: EventKind.readState,
      tags: [
        ['d', 'channel-filter'],
      ],
      content: content,
      secretKey: nostr.Nip19.decode(payload: keys.nsec).data,
      createdAt: at,
      verify: false,
    ).toMap(),
  );
  ChannelFilterManager manager({
    ChannelFilterStore? startup,
    Future<List<NostrEvent>> Function()? fetch,
  }) => ChannelFilterManager(
    pubkey: keys.public,
    crypto: crypto,
    cached: startup,
    cache: cached.add,
    isGroupLive: (_) => live,
    fetch:
        fetch ??
        () async {
          if (failRead) throw StateError('offline');
          return history;
        },
    publish: (content, at) async {
      publishes++;
      if (failPublish) throw StateError('rejected');
      if (retainPublish) history = [signed(content, at)];
    },
  );
  test(
    'startup cache is display-only; failed reads cannot enable writes or publish',
    () async {
      failRead = true;
      final subject = manager(
        startup: ChannelFilterStore.empty().edit(
          'channels',
          ChannelFilterMode.unreads,
        ),
      );
      await subject.refresh();
      expect(subject.store.modeFor('channels'), ChannelFilterMode.unreads);
      expect(subject.ready, isFalse);
      await expectLater(
        subject.save('channels', ChannelFilterMode.all),
        throwsStateError,
      );
      expect(publishes, 0);
      subject.dispose();
    },
  );
  test(
    'failed save rolls back; explicit retry confirms mode and duration together',
    () async {
      final subject = manager();
      await subject.refresh();
      failPublish = true;
      await expectLater(
        subject.save(
          'channels',
          ChannelFilterMode.active,
          windowSeconds: 604800,
        ),
        throwsStateError,
      );
      expect(subject.store.modeFor('channels'), ChannelFilterMode.all);
      expect(subject.busy, isFalse);
      failPublish = false;
      await subject.save(
        'channels',
        ChannelFilterMode.active,
        windowSeconds: 604800,
      );
      expect(subject.store.modeFor('channels'), ChannelFilterMode.active);
      expect(subject.store.windowFor('channels'), 604800);
      expect(cached.last.windowFor('channels'), 604800);
      subject.dispose();
    },
  );
  test(
    'signed malformed siblings are salvaged and explicitly saved as a repaired head',
    () async {
      final malformed = {
        'version': 1,
        'groups': {'channels': 'active', 'forums': 'invalid', 'dms': 'active'},
        'activeWindowSeconds': {'channels': 604800, 'forums': -1},
        'future': {'keep': true},
        'meta': {
          'v': 1,
          'unknownSibling': {'rejectOnDesktop': true},
          'g': {
            'channels': [12, '0000000000000000', 'active'],
            'forums': ['bad', '0000000000000000', 'unreads'],
            'dms': [13, '0000000000000000', null],
          },
        },
      };
      history = [signed(crypto.encrypt(jsonEncode(malformed)), 10)];
      // A cached sibling must never be promoted into the repair.
      final subject = manager(
        startup: ChannelFilterStore.empty().edit(
          'starred',
          ChannelFilterMode.unreads,
        ),
      );
      await subject.refresh();
      expect(subject.ready, isTrue);
      expect(subject.store.modeFor('channels'), ChannelFilterMode.active);
      expect(subject.store.modeFor('forums'), ChannelFilterMode.all);
      expect(subject.store.modeFor('dms'), ChannelFilterMode.all);
      expect(subject.store.modeFor('starred'), ChannelFilterMode.all);
      expect(publishes, 0);
      await subject.save('forums', ChannelFilterMode.unreads);
      final repaired = jsonDecode(crypto.decrypt(history.single.content));
      expect(
        repaired['meta']['g']['channels'],
        (malformed['meta'] as Map)['g']['channels'],
      );
      expect(repaired['meta']['g']['dms'][2], isNull);
      expect(repaired['activeWindowSeconds'], {'channels': 604800});
      expect(repaired['groups'], {'channels': 'active', 'forums': 'unreads'});
      expect(repaired['future'], malformed['future']);
      expect((repaired['meta'] as Map).keys, unorderedEquals(['v', 'g']));
      expect(subject.ready, isTrue);
      expect(cached.last.toJson(), repaired);
      subject.dispose();
    },
  );

  test('OK without matching readback is failure', () async {
    final subject = manager();
    await subject.refresh();
    retainPublish = false;
    await expectLater(
      subject.save('channels', ChannelFilterMode.active, windowSeconds: 86400),
      throwsStateError,
    );
    expect(subject.store.modeFor('channels'), ChannelFilterMode.all);
    expect(subject.error, isNotNull);
    subject.dispose();
  });
  test(
    'preflight preserves another device edit and rejects removed-section retry',
    () async {
      final subject = manager();
      await subject.refresh();
      history = [
        signed(
          crypto.encrypt(
            jsonEncode(
              ChannelFilterStore.empty()
                  .edit(
                    'forums',
                    ChannelFilterMode.active,
                    windowSeconds: 86400,
                  )
                  .toJson(),
            ),
          ),
          10,
        ),
      ];
      await subject.save('channels', ChannelFilterMode.unreads);
      expect(subject.store.modeFor('forums'), ChannelFilterMode.active);
      expect(subject.store.windowFor('forums'), 86400);
      live = false;
      await expectLater(
        subject.save('section:gone', ChannelFilterMode.unreads),
        throwsStateError,
      );
      expect(publishes, 1);
      subject.dispose();
    },
  );
  test(
    'disposed read never enables writes or persists a different session',
    () async {
      final pending = Completer<List<NostrEvent>>();
      final subject = manager(fetch: () => pending.future);
      final loading = subject.refresh();
      subject.dispose();
      pending.complete([]);
      await loading;
      expect(cached, isEmpty);
      expect(subject.ready, isFalse);
    },
  );
}
