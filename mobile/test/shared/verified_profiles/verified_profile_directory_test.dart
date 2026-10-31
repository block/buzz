import 'dart:async';

import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/verified_profiles/verified_profile.dart';
import 'package:buzz/shared/verified_profiles/verified_profile_directory.dart';
import 'package:buzz/shared/verified_profiles/verified_profile_discovery_fetch.dart';
import 'package:buzz/shared/verified_profiles/verified_profile_proof.dart';
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/fake_relay.dart';
import 'verified_profile_fixture.dart';

class Harness {
  Harness(this.async) {
    container = ProviderContainer(
      overrides: [
        ...relay.overrides,
        verifiedProfileDiscoveryFetchProvider.overrideWith(
          (ref) => () async {
            fetches++;
            final next = discovery;
            if (next is Exception) throw next;
            return next as VerifiedProfileDiscovery;
          },
        ),
        verifiedProfileProofCheckerProvider.overrideWithValue(
          (events) async => [for (final e in events) hasVerifiedEventProof(e)],
        ),
      ],
    );
  }

  final FakeAsync async;
  final relay = FakeRelay();
  late final ProviderContainer container;
  Object discovery = VerifiedProfilesEnabled(authority);
  int fetches = 0;

  Map<String, VerifiedProfile> get profiles =>
      container.read(verifiedProfilesProvider);
  void settle([int ms = 100]) => async.elapse(Duration(milliseconds: ms));

  void start(Iterable<String> subjects) {
    container.listen(verifiedProfilesProvider, (_, _) {});
    settle();
    container.read(verifiedProfilesProvider.notifier).track(subjects);
    settle();
  }

  Iterable<Map<String, dynamic>> get dFilters => relay.reqs
      .expand((req) => req.skip(2).cast<Map<String, dynamic>>())
      .where((f) => (f['kinds'] as List).contains(verifiedProfileKind));
}

void journey(String name, void Function(Harness h) body) {
  test(name, () {
    fakeAsync((async) {
      final h = Harness(async);
      addTearDown(h.container.dispose);
      body(h);
    });
  });
}

void main() {
  journey('no marks until discovery succeeds and advertises public_profile', (
    h,
  ) {
    h.relay.seed([signedProjection(profileTags(subjectA))]);
    h.discovery = const FormatException('malformed');
    h.start([subjectA]);
    expect(h.fetches, greaterThan(0));
    expect(h.profiles, isEmpty);
    expect(h.dFilters, isEmpty, reason: 'nothing subscribed without discovery');

    h.discovery = VerifiedProfilesEnabled(authority);
    h.async.elapse(const Duration(minutes: 5));
    expect(h.profiles[subjectA]?.displayName, 'Ada Lovelace');
    expect(h.profiles[subjectA]?.checked, isTrue);
    final filter = h.dFilters.single;
    expect(filter['authors'], [authority.pubkey]);
    expect(filter['#d'], [subjectA]);
  });

  journey('forged and impostor projections are ignored', (h) {
    h.relay.seed([
      signedProjection(profileTags(subjectA), author: relayKeysOther()),
    ]);
    h.start([subjectA]);
    expect(h.profiles, isEmpty);
  });

  journey('subjects are batched in groups of at most 128', (h) {
    final subjects = [
      for (var i = 0; i < 130; i++) i.toRadixString(16).padLeft(64, '0'),
    ];
    h.start(subjects);
    final sizes = [for (final f in h.dFilters) (f['#d'] as List).length];
    expect(sizes..sort(), [2, 128]);
  });

  journey('live updates, retractions and ordering apply on the subscription', (
    h,
  ) {
    h.relay.seed([signedProjection(profileTags(subjectA), createdAt: 2000)]);
    h.start([subjectA]);
    expect(h.profiles[subjectA]?.displayName, 'Ada Lovelace');

    h.relay.deliver(
      signedProjection(profileTags(subjectA, name: 'Ada K'), createdAt: 2100),
    );
    h.settle();
    expect(h.profiles[subjectA]?.displayName, 'Ada K');

    // An older replay never overwrites a newer record.
    h.relay.deliver(
      signedProjection(profileTags(subjectA, name: 'Old'), createdAt: 1500),
    );
    h.settle();
    expect(h.profiles[subjectA]?.displayName, 'Ada K');

    // A bad-signature impostor is dropped before it can supersede.
    final good = signedProjection(profileTags(subjectA), createdAt: 9000);
    h.relay.deliver(
      NostrEvent(
        id: good.id,
        pubkey: good.pubkey,
        createdAt: good.createdAt,
        kind: good.kind,
        tags: good.tags,
        content: good.content,
        sig: '0' * 128,
      ),
    );
    h.settle();
    expect(h.profiles[subjectA]?.displayName, 'Ada K');

    h.relay.deliver(
      signedProjection(retractionTags(subjectA), createdAt: 2200),
    );
    h.settle();
    expect(h.profiles, isEmpty);
  });

  journey('disconnect keeps cached records, reconnect reconciles them', (h) {
    h.relay.seed([
      signedProjection(profileTags(subjectA), createdAt: 2000),
      signedProjection(profileTags(subjectB, name: 'Bob', username: 'bob')),
    ]);
    h.start([subjectA, subjectB]);
    expect(h.profiles.values.every((p) => p.checked), isTrue);
    final firstSockets = h.relay.sockets.length;

    // Missed while offline: A is retracted. B vanishes from a response that
    // is full (two events for A), which cannot prove B has no projection.
    h.relay.drop();
    h.settle(10);
    expect(h.profiles.length, 2, reason: 'history is kept while offline');
    expect(h.profiles.values.any((p) => p.checked), isFalse);
    h.relay.stored
      ..removeWhere((e) => e.getTagValue('d') == subjectB)
      ..add(signedProjection(retractionTags(subjectA), createdAt: 2500))
      ..add(signedProjection(retractionTags(subjectA), createdAt: 2400));
    h.async.elapse(const Duration(seconds: 5));
    expect(h.relay.sockets.length, greaterThan(firstSockets));
    expect(h.profiles.containsKey(subjectA), isFalse);
    expect(
      h.profiles[subjectB]?.checked,
      isFalse,
      reason: 'possibly truncated',
    );
  });

  journey('a short response proves absence and clears the record', (h) {
    h.relay.seed([
      signedProjection(profileTags(subjectA)),
      signedProjection(profileTags(subjectB, name: 'Bob', username: 'bob')),
    ]);
    h.start([subjectA, subjectB]);
    h.relay.drop();
    h.relay.stored.removeWhere((e) => e.getTagValue('d') == subjectB);
    h.async.elapse(const Duration(seconds: 5));
    expect(h.profiles.keys, [subjectA]);
    expect(h.profiles[subjectA]?.checked, isTrue);
  });

  journey('discovery refresh: disable hides cache, failure keeps state', (h) {
    h.relay.seed([signedProjection(profileTags(subjectA))]);
    h.start([subjectA]);
    expect(h.profiles, isNotEmpty);

    h.discovery = const FormatException('garbled');
    h.async.elapse(const Duration(minutes: 5));
    expect(h.profiles[subjectA], isNotNull);
    expect(h.profiles[subjectA]?.checked, isFalse, reason: 'not fresh');

    h.discovery = const VerifiedProfilesDisabled();
    h.async.elapse(const Duration(minutes: 5));
    expect(h.profiles, isEmpty);

    h.discovery = VerifiedProfilesEnabled(authority);
    h.async.elapse(const Duration(minutes: 5));
    expect(h.profiles[subjectA]?.displayName, 'Ada Lovelace');
  });

  journey('a changed self key discards the cache', (h) {
    h.relay.seed([signedProjection(profileTags(subjectA))]);
    h.start([subjectA]);
    expect(h.profiles, isNotEmpty);
    h.discovery = VerifiedProfilesEnabled(
      VerifiedProfileAuthority(pubkey: 'c' * 64, multipleIssuers: false),
    );
    h.async.elapse(const Duration(minutes: 5));
    expect(h.profiles, isEmpty);
  });

  journey('profiles hydrated through resolve are tracked', (h) {
    h.relay.seed([signedProjection(profileTags(subjectA))]);
    h.start(const []);
    h.container.listen(userCacheProvider, (_, _) {});
    unawaited(h.container.read(userCacheProvider.notifier).resolve([subjectA]));
    h.settle();
    expect(h.dFilters.single['#d'], [subjectA]);
    expect(h.profiles[subjectA]?.displayName, 'Ada Lovelace');
  });
}
