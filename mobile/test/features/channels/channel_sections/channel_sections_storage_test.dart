import 'package:buzz/features/channels/channel_sections/channel_sections_storage.dart';
import 'package:flutter_test/flutter_test.dart';

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
}
