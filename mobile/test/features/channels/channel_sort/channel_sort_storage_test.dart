import 'dart:convert';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_sort/channel_sort_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  group('ChannelSortStore JSON', () {
    test('round-trips desktop wire format and drops invalid modes', () {
      final store = ChannelSortStore(
        groups: {
          'channels': ChannelSortMode.recent,
          'dms': ChannelSortMode.alpha,
        },
      );
      expect(store.toJson()['groups'], {'channels': 'recent', 'dms': 'alpha'});
      expect(ChannelSortStore.fromJson(store.toJson()).groups, store.groups);
      expect(
        ChannelSortStore.fromJson({
          'version': 1,
          'groups': {'channels': 'recent', 'starred': 'bogus'},
        }).groups,
        {'channels': ChannelSortMode.recent},
      );
    });
  });

  group('ChannelSortStorage', () {
    test('normalizes relay scope and isolates communities', () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final storage = ChannelSortStorage(prefs);
      final store = ChannelSortStore(
        groups: {'channels': ChannelSortMode.recent},
      );
      storage.write('pk', ' WSS://Relay.Example/ ', store);
      expect(storage.read('pk', 'wss://relay.example').groups, store.groups);
      expect(storage.read('pk', 'wss://other.example').groups, isEmpty);
    });

    test('migrates legacy unscoped cache into the first relay scope', () async {
      SharedPreferences.setMockInitialValues({
        legacyChannelSortKey('pk'): jsonEncode({
          'version': 1,
          'groups': {'dms': 'recent'},
        }),
      });
      final prefs = await SharedPreferences.getInstance();
      final storage = ChannelSortStorage(prefs);
      expect(storage.read('pk', 'wss://one').groups, {
        'dms': ChannelSortMode.recent,
      });
      expect(prefs.getString(channelSortKey('pk', 'wss://one')), isNotNull);
      expect(prefs.getString(legacyChannelSortKey('pk')), isNull);
      expect(storage.read('pk', 'wss://two').groups, isEmpty);
    });

    test('ignores corrupt and unsupported payloads', () async {
      SharedPreferences.setMockInitialValues({
        channelSortKey('pk', 'wss://one'): 'nope',
        channelSortKey('pk', 'wss://two'): '{"version":2,"groups":{}}',
      });
      final prefs = await SharedPreferences.getInstance();
      final storage = ChannelSortStorage(prefs);
      expect(storage.read('pk', 'wss://one').groups, isEmpty);
      expect(storage.read('pk', 'wss://two').groups, isEmpty);
    });
  });

  test('prunes orphaned section modes but keeps fixed groups', () {
    final store = ChannelSortStore(
      groups: {
        'channels': ChannelSortMode.recent,
        'section:live': ChannelSortMode.recent,
        'section:dead': ChannelSortMode.alpha,
      },
    );
    expect(stripOrphanedSectionModes(store, ['live']).groups.keys, [
      'channels',
      'section:live',
    ]);
  });

  group('sortChannelsForList', () {
    Channel channel(String id, String name, {DateTime? lastMessageAt}) =>
        Channel(
          id: id,
          name: name,
          channelType: 'stream',
          visibility: 'open',
          description: '',
          createdBy: 'pk',
          createdAt: DateTime.utc(2026),
          memberCount: 1,
          lastMessageAt: lastMessageAt,
        );

    test('alpha matches desktop code-unit collation and id tie-break', () {
      final sorted = sortChannelsForList([
        channel('2', 'zeta'),
        channel('3', 'Alpha'),
        channel('1', 'alpha'),
        channel('4', 'Éclair'),
      ], ChannelSortMode.alpha);
      expect(sorted.map((c) => c.id), ['1', '3', '2', '4']);
    });

    test('recent puts newest first and quiet channels alpha last', () {
      final now = DateTime.utc(2026, 7, 25);
      final sorted = sortChannelsForList([
        channel('a', 'quiet-z'),
        channel(
          'b',
          'old',
          lastMessageAt: now.subtract(const Duration(days: 2)),
        ),
        channel('c', 'new', lastMessageAt: now),
        channel('d', 'quiet-a'),
      ], ChannelSortMode.recent);
      expect(sorted.map((c) => c.id), ['c', 'b', 'd', 'a']);
    });
  });
}
