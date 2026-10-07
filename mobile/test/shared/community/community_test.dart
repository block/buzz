import 'package:buzz/shared/community/community.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('existing records default to device authentication', () {
    final community = Community.fromJson({
      'id': 'one',
      'name': 'Buzz',
      'relayUrl': 'https://relay.test',
      'addedAt': '2026-08-05T00:00:00.000Z',
    });

    expect(
      community.sensitiveActionPolicy,
      SensitiveActionPolicy.disabledByUser,
    );
  });

  test('sensitive action policy round trips', () {
    final community = Community(
      id: 'one',
      name: 'Buzz',
      relayUrl: 'https://relay.test',
      sensitiveActionPolicy: SensitiveActionPolicy.enabled,
      addedAt: DateTime.utc(2026, 8, 5),
    );

    expect(
      Community.fromJson(community.toJson()).sensitiveActionPolicy,
      SensitiveActionPolicy.enabled,
    );
  });
}
