import 'dart:convert';

import 'package:buzz/features/channels/channel_sections/channel_sections_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  group('ChannelSection icon', () {
    test('round-trips through fromJson and toJson', () {
      const original = ChannelSection(
        id: 'section-1',
        name: 'Friends',
        icon: ':party_parrot:',
        order: 0,
      );

      final roundTrip = ChannelSection.fromJson(original.toJson());

      expect(roundTrip.icon, ':party_parrot:');
      expect(roundTrip.toJson(), original.toJson());
    });

    test('missing icon remains absent', () {
      final section = ChannelSection.fromJson({
        'id': 'section-1',
        'name': 'Friends',
        'order': 0,
      });

      expect(section.icon, isNull);
      expect(section.toJson(), isNot(contains('icon')));
    });

    test('empty icon round-trips', () {
      final section = ChannelSection.fromJson({
        'id': 'section-1',
        'name': 'Friends',
        'icon': '',
        'order': 0,
      });

      expect(section.icon, isEmpty);
      expect(section.toJson()['icon'], '');
    });
  });

  group('ChannelSectionsStorage', () {
    test('normalizes relay scope and isolates communities', () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final storage = ChannelSectionsStorage(prefs);
      final store = ChannelSectionStore(
        sections: const [
          ChannelSection(id: 's1', name: 'production', order: 0),
        ],
      );
      storage.write('pk', ' WSS://Relay.Example/ ', store);
      expect(
        storage.read('pk', 'wss://relay.example').sections.single.name,
        'production',
      );
      expect(storage.read('pk', 'wss://other.example').sections, isEmpty);
    });

    test('migrates legacy unscoped cache into the first relay scope', () async {
      SharedPreferences.setMockInitialValues({
        legacyChannelSectionsKey('pk'): jsonEncode({
          'version': 1,
          'sections': [
            {'id': 's1', 'name': 'estimates', 'order': 0},
          ],
          'assignments': {'chan-1': 's1'},
        }),
      });
      final prefs = await SharedPreferences.getInstance();
      final storage = ChannelSectionsStorage(prefs);
      final migrated = storage.read('pk', 'wss://one');
      expect(migrated.sections.single.name, 'estimates');
      expect(migrated.assignments, {'chan-1': 's1'});
      expect(prefs.getString(channelSectionsKey('pk', 'wss://one')), isNotNull);
      expect(prefs.getString(legacyChannelSectionsKey('pk')), isNull);
      // Second community must not inherit the migrated legacy blob.
      expect(storage.read('pk', 'wss://two').sections, isEmpty);
    });

    test('ignores corrupt and unsupported payloads', () async {
      SharedPreferences.setMockInitialValues({
        channelSectionsKey('pk', 'wss://one'): 'nope',
        channelSectionsKey('pk', 'wss://two'): '{"version":2,"sections":[]}',
      });
      final prefs = await SharedPreferences.getInstance();
      final storage = ChannelSectionsStorage(prefs);
      expect(storage.read('pk', 'wss://one').sections, isEmpty);
      expect(storage.read('pk', 'wss://two').sections, isEmpty);
    });

    test('empty legacy store does not claim first relay', () async {
      SharedPreferences.setMockInitialValues({
        legacyChannelSectionsKey('pk'): jsonEncode({
          'version': 1,
          'sections': <Map<String, dynamic>>[],
          'assignments': <String, String>{},
        }),
      });
      final prefs = await SharedPreferences.getInstance();
      final storage = ChannelSectionsStorage(prefs);
      expect(storage.read('pk', 'wss://one').sections, isEmpty);
      // Empty legacy is not migrated, so key may still exist — but scoped
      // key must not be written for an empty claim.
      expect(prefs.getString(channelSectionsKey('pk', 'wss://one')), isNull);
    });
  });
}
