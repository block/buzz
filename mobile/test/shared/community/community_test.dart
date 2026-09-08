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
    expect(community.pushLeaseInstallationId, isNull);
  });

  test('new community gets a unique canonical push lease address id', () {
    final first = Community.create(name: 'One', relayUrl: 'https://relay.test');
    final second = Community.create(
      name: 'Two',
      relayUrl: 'https://relay.test',
    );

    expect(first.pushLeaseInstallationId, matches(RegExp(r'^[0-9a-f]{32}$')));
    expect(
      second.pushLeaseInstallationId,
      isNot(first.pushLeaseInstallationId),
    );
  });

  test('community settings round trip', () {
    final community = Community(
      id: 'one',
      name: 'Buzz',
      relayUrl: 'https://relay.test',
      sensitiveActionPolicy: SensitiveActionPolicy.enabled,
      pushLeaseInstallationId: 'a' * 32,
      starterSetupIncomplete: true,
      addedAt: DateTime.utc(2026, 8, 5),
    );

    final roundTrip = Community.fromJson(community.toJson());
    expect(roundTrip.sensitiveActionPolicy, SensitiveActionPolicy.enabled);
    expect(roundTrip.starterSetupIncomplete, isTrue);
    expect(roundTrip.pushLeaseInstallationId, 'a' * 32);
  });

  test('malformed stored push lease address id is rejected', () {
    expect(
      () => Community.fromJson({
        'id': 'one',
        'name': 'Buzz',
        'relayUrl': 'https://relay.test',
        'pushLeaseInstallationId': 'not-canonical',
        'addedAt': '2026-08-05T00:00:00.000Z',
      }),
      throwsFormatException,
    );
  });
}
