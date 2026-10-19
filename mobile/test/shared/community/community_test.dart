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
    expect(community.pushNotificationsEnabled, isTrue);
    expect(community.pushSubscriptionState.pendingTombstoneGeneration, isNull);
  });

  test('new community gets a unique canonical push lease address id', () {
    final first = Community.create(name: 'One', relayUrl: 'https://relay.test');
    final second = Community.create(
      name: 'Two',
      relayUrl: 'https://relay.test',
    );

    expect(first.pushNotificationsEnabled, isTrue);
    expect(Community.fromJson(first.toJson()).pushNotificationsEnabled, isTrue);
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

  for (final enabled in [true, false]) {
    test('saved push preference $enabled survives loading and copying', () {
      final community = Community.fromJson({
        'id': 'one',
        'name': 'Buzz',
        'relayUrl': 'https://relay.test',
        'pushNotificationsEnabledV2': enabled,
        'pushNotificationsEnabled': !enabled,
        'addedAt': '2026-08-05T00:00:00.000Z',
      });
      expect(community.pushNotificationsEnabled, enabled);
      expect(
        community.copyWith(name: 'Renamed').pushNotificationsEnabled,
        enabled,
      );
      expect(
        Community.fromJson(community.toJson()).pushNotificationsEnabled,
        enabled,
      );
      expect(
        community.pushSubscriptionState.pendingTombstoneGeneration,
        isNull,
      );
    });
  }

  test('previous preference values are intentionally not preserved', () {
    for (final previous in [false, true]) {
      final saved = Community.create(
        name: 'Team',
        relayUrl: 'https://relay.test',
      ).toJson()..remove('pushNotificationsEnabledV2');
      saved['pushNotificationsEnabled'] = previous;
      final community = Community.fromJson(saved);
      expect(community.pushNotificationsEnabled, isTrue);
      expect(community.toJson(), isNot(contains('pushNotificationsEnabled')));
    }
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
