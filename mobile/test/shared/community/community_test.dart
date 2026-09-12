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

  group('value equality', () {
    Community sample() => Community(
      id: 'one',
      name: 'Buzz',
      relayUrl: 'https://relay.test',
      pubkey: 'a' * 64,
      nsec: 'nsec1test',
      pushNotificationsEnabled: true,
      pushLeaseInstallationId: '0' * 32,
      addedAt: DateTime.utc(2026, 8, 5),
    );

    test('two separately built identical communities compare equal', () {
      final a = sample();
      final b = sample();

      expect(identical(a, b), isFalse);
      expect(a, b);
      expect(a.hashCode, b.hashCode);
    });

    test('a copyWith that changes nothing compares equal', () {
      // This is the regression. reservePushLeaseGeneration saves the community
      // on every publish attempt; without ==, each save emitted a new object,
      // rebuilt every watcher of activeCommunityProvider — including
      // ReadStateNotifier, which disposes and recreates its manager in build()
      // — and the resulting relay churn failed the publish that triggered it.
      final a = sample();
      expect(a.copyWith(), a);
    });

    test('each field participates in equality', () {
      final a = sample();

      expect(a.copyWith(name: 'Other'), isNot(a));
      expect(a.copyWith(relayUrl: 'https://other.test'), isNot(a));
      expect(a.copyWith(pubkey: 'b' * 64), isNot(a));
      expect(a.copyWith(nsec: 'nsec1other'), isNot(a));
      expect(a.copyWith(pushNotificationsEnabled: false), isNot(a));
      expect(a.copyWith(starterSetupIncomplete: true), isNot(a));
      expect(
        a.copyWith(sensitiveActionPolicy: SensitiveActionPolicy.enabled),
        isNot(a),
      );
    });

    test('a changed push subscription state is not equal', () {
      final a = sample();
      final b = a.copyWith(
        pushSubscriptionState: a.pushSubscriptionState.withReservedGeneration(
          (a.pushSubscriptionState.generationCursor ?? 0) + 1,
        ),
      );

      expect(b, isNot(a));
    });
  });
}
