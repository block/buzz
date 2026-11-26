import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:buzz/features/channels/channel_filter/channel_filter_manager.dart';
import 'package:buzz/features/channels/channel_filter/channel_filter_store.dart';
import 'package:buzz/features/channels/channel_sort/channel_sort_manager.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final fixture = jsonDecode(
    File('test/fixtures/channel_filter_desktop.json').readAsStringSync(),
  );
  final keys = nostr.Keys(fixture['testSecretHex'] as String);
  final crypto = ChannelSortCrypto(keys.nsec, keys.public);
  NostrEvent signed(String content, int at) => NostrEvent.fromJson(
    nostr.Event.from(
      kind: EventKind.readState,
      tags: [
        ['d', 'channel-filter'],
      ],
      content: content,
      secretKey: fixture['testSecretHex'] as String,
      createdAt: at,
      verify: false,
    ).toMap(),
  );

  test(
    'desktop encrypted golden head is adopted and preserved in mobile writes',
    () async {
      var history = [
        NostrEvent.fromJson((fixture['head'] as Map).cast<String, dynamic>()),
      ];
      final manager = ChannelFilterManager(
        pubkey: keys.public,
        crypto: crypto,
        fetch: () async => history,
        cache: (_) {},
        publish: (content, at) async => history = [signed(content, at)],
      );
      await manager.refresh();
      expect(manager.ready, isTrue);
      expect(manager.store.toJson(), fixture['plaintext']);
      expect(manager.store.modeFor('channels'), ChannelFilterMode.active);
      expect(manager.store.windowFor('channels'), 604800);
      await manager.save(
        'forums',
        ChannelFilterMode.active,
        windowSeconds: 86400,
      );
      final saved = jsonDecode(crypto.decrypt(history.single.content));
      final mobileGolden = NostrEvent.fromJson(
        (fixture['mobileHead'] as Map).cast<String, dynamic>(),
      );
      nostr.Event.fromJson(jsonEncode(mobileGolden.toJson()));
      final mobilePayload = jsonDecode(crypto.decrypt(mobileGolden.content));
      expect(saved['groups'], mobilePayload['groups']);
      expect(
        saved['activeWindowSeconds'],
        mobilePayload['activeWindowSeconds'],
      );
      expect(saved['groups'], {'channels': 'active', 'forums': 'active'});
      expect(saved['activeWindowSeconds'], {
        'channels': 604800,
        'forums': 86400,
      });
      expect(
        saved['meta']['g']['channels'],
        fixture['plaintext']['meta']['g']['channels'],
      );
      manager.dispose();
    },
  );

  test(
    'crossed edits converge to one whole-record winner with mode and duration together',
    () async {
      var head = NostrEvent.fromJson(
        (fixture['head'] as Map).cast<String, dynamic>(),
      );
      final initial = head;
      var preflights = 0;
      final bothRead = Completer<void>();
      final bothPublished = Completer<void>();
      final published = <NostrEvent>[];
      ChannelFilterManager device() => ChannelFilterManager(
        pubkey: keys.public,
        crypto: crypto,
        cache: (_) {},
        fetch: () async {
          if (preflights < 2) {
            preflights++;
            if (preflights == 2) bothRead.complete();
            await bothRead.future;
            return [initial];
          }
          return [head];
        },
        publish: (content, at) async {
          published.add(signed(content, at));
          if (published.length == 2) {
            // NIP-01: newest created_at, then lowest event ID wins.
            published.sort(
              (a, b) => a.createdAt != b.createdAt
                  ? b.createdAt.compareTo(a.createdAt)
                  : a.id.compareTo(b.id),
            );
            head = published.first;
            bothPublished.complete();
          }
          await bothPublished.future;
        },
      )..ready = true;
      final a = device();
      final b = device();
      final results = await Future.wait([
        a
            .save('channels', ChannelFilterMode.active, windowSeconds: 86400)
            .then((_) => true, onError: (_) => false),
        b
            .save('channels', ChannelFilterMode.unreads)
            .then((_) => true, onError: (_) => false),
      ]);
      expect(results.where((success) => success).length, 1);
      await a.refresh();
      await b.refresh();
      final winner = jsonDecode(crypto.decrypt(head.content));
      expect(a.store.toJson(), winner);
      expect(b.store.toJson(), winner);
      expect(
        a.store.modeFor('channels') == ChannelFilterMode.active
            ? a.store.windowFor('channels')
            : b.store.windowFor('channels'),
        a.store.modeFor('channels') == ChannelFilterMode.active
            ? 86400
            : 604800,
      );
      a.dispose();
      b.dispose();
    },
  );
}
