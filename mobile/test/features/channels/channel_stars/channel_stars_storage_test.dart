import 'package:flutter_test/flutter_test.dart';
import 'package:buzz/features/channels/channel_stars/channel_stars_storage.dart';

/// Tests for [ChannelStarStore] parsing and [mergeStores] last-writer-wins.
void main() {
  group('ChannelStarStore.fromJson', () {
    test('parses a valid payload', () {
      final store = ChannelStarStore.fromJson({
        'version': 1,
        'channels': {
          'chan-1': {'starred': true, 'updatedAt': 1000},
          'chan-2': {'starred': false, 'updatedAt': 2000},
        },
      });
      expect(store.channels.length, 2);
      expect(store.channels['chan-1']!.starred, isTrue);
      expect(store.channels['chan-1']!.updatedAt, 1000);
      expect(store.channels['chan-2']!.starred, isFalse);
    });

    test('drops malformed entries (missing/wrong-typed fields)', () {
      final store = ChannelStarStore.fromJson({
        'version': 1,
        'channels': {
          'no-starred': {'updatedAt': 1000},
          'no-updated-at': {'starred': true},
          'starred-wrong-type': {'starred': 'yes', 'updatedAt': 1000},
          'updated-wrong-type': {'starred': true, 'updatedAt': 'now'},
          'valid': {'starred': true, 'updatedAt': 500},
        },
      });
      expect(store.channels.keys, ['valid']);
      expect(store.channels['valid']!.updatedAt, 500);
    });

    test('empty / missing channels yields empty store', () {
      expect(ChannelStarStore.fromJson({'version': 1}).channels, isEmpty);
      expect(
        ChannelStarStore.fromJson({'version': 1, 'channels': {}}).channels,
        isEmpty,
      );
    });

    test('round-trips through toJson', () {
      const original = ChannelStarStore(
        channels: {'c': ChannelStarEntry(starred: true, updatedAt: 42)},
      );
      final round = ChannelStarStore.fromJson(original.toJson());
      expect(round.channels['c']!.starred, isTrue);
      expect(round.channels['c']!.updatedAt, 42);
    });
  });

  group('mergeStores (per-channel max-updatedAt)', () {
    test('union of non-overlapping channels', () {
      final merged = mergeStores(
        const ChannelStarStore(
          channels: {'a': ChannelStarEntry(starred: true, updatedAt: 100)},
        ),
        const ChannelStarStore(
          channels: {'b': ChannelStarEntry(starred: false, updatedAt: 200)},
        ),
      );
      expect(merged.channels.keys.toSet(), {'a', 'b'});
    });

    test('higher updatedAt wins (remote newer)', () {
      final merged = mergeStores(
        const ChannelStarStore(
          channels: {'a': ChannelStarEntry(starred: false, updatedAt: 100)},
        ),
        const ChannelStarStore(
          channels: {'a': ChannelStarEntry(starred: true, updatedAt: 200)},
        ),
      );
      expect(merged.channels['a']!.starred, isTrue);
      expect(merged.channels['a']!.updatedAt, 200);
    });

    test('higher updatedAt wins (local newer)', () {
      final merged = mergeStores(
        const ChannelStarStore(
          channels: {'a': ChannelStarEntry(starred: true, updatedAt: 300)},
        ),
        const ChannelStarStore(
          channels: {'a': ChannelStarEntry(starred: false, updatedAt: 100)},
        ),
      );
      expect(merged.channels['a']!.starred, isTrue);
      expect(merged.channels['a']!.updatedAt, 300);
    });

    test('unstar with higher updatedAt overrides star', () {
      final merged = mergeStores(
        const ChannelStarStore(
          channels: {'a': ChannelStarEntry(starred: true, updatedAt: 100)},
        ),
        const ChannelStarStore(
          channels: {'a': ChannelStarEntry(starred: false, updatedAt: 999)},
        ),
      );
      expect(merged.channels['a']!.starred, isFalse);
      expect(merged.channels['a']!.updatedAt, 999);
    });

    test('both empty yields empty', () {
      final merged = mergeStores(
        const ChannelStarStore(),
        const ChannelStarStore(),
      );
      expect(merged.channels, isEmpty);
    });
  });
}
