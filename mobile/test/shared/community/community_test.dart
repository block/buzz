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
    expect(community.starterSetupIncomplete, isFalse);
  });

  test('community settings round trip', () {
    final community = Community(
      id: 'one',
      name: 'Buzz',
      relayUrl: 'https://relay.test',
      sensitiveActionPolicy: SensitiveActionPolicy.enabled,
      starterSetupIncomplete: true,
      addedAt: DateTime.utc(2026, 8, 5),
    );

    final roundTrip = Community.fromJson(community.toJson());
    expect(roundTrip.sensitiveActionPolicy, SensitiveActionPolicy.enabled);
    expect(roundTrip.starterSetupIncomplete, isTrue);
  });
}
