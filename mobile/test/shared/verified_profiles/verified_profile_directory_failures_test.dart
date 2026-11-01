import 'dart:async';

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

/// A relay that answers every verified-profile REQ with [history] then EOSE,
/// ignoring the filter's limit.
class _ScriptedSession extends RelaySessionNotifier {
  List<NostrEvent> history = const [];

  /// When false, the test delivers events and statuses itself.
  bool autoEose = true;
  int subscriptions = 0;
  int unsubscribed = 0;
  void Function(NostrEvent)? emit;
  void Function(RelaySubscriptionStatus)? status;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<void Function()> subscribeWithStatus(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
    required void Function(RelaySubscriptionStatus status) onStatusChanged,
  }) async {
    subscriptions++;
    emit = onEvent;
    status = onStatusChanged;
    final events = history;
    if (!autoEose) return () => unsubscribed++;
    Future.microtask(() {
      for (final event in events) {
        onEvent(event);
      }
      onStatusChanged(RelaySubscriptionStatus.ready);
    });
    return () => unsubscribed++;
  }
}

void main() {
  late _ScriptedSession session;
  late ProviderContainer container;
  late int proofCalls;
  late int provenEvents;
  late bool proofThrows;
  Completer<void>? proofGate;

  void setUpContainer() {
    session = _ScriptedSession();
    proofCalls = 0;
    provenEvents = 0;
    proofThrows = false;
    proofGate = null;
    container = ProviderContainer(
      overrides: [
        // FakeRelay supplies config and auth; the session is scripted here.
        ...FakeRelay().overrides.skip(1),
        relaySessionProvider.overrideWith(() => session),
        verifiedProfileDiscoveryFetchProvider.overrideWith(
          (ref) =>
              () async => VerifiedProfilesEnabled(authority),
        ),
        verifiedProfileProofCheckerProvider.overrideWithValue((events) async {
          proofCalls++;
          provenEvents += events.length;
          if (proofThrows) throw StateError('isolate failed');
          await proofGate?.future;
          return [for (final e in events) hasVerifiedEventProof(e)];
        }),
      ],
    );
    addTearDown(container.dispose);
  }

  Map<String, VerifiedProfile> start(
    FakeAsync async, [
    List<String>? subjects,
  ]) {
    container.listen(verifiedProfilesProvider, (_, _) {});
    async.elapse(const Duration(milliseconds: 100));
    container
        .read(verifiedProfilesProvider.notifier)
        .track(subjects ?? [subjectA]);
    async.elapse(const Duration(milliseconds: 100));
    return container.read(verifiedProfilesProvider);
  }

  test('a flood before EOSE fails the batch without verifying it', () {
    fakeAsync((async) {
      setUpContainer();
      session.history = [
        for (var i = 0; i < 50; i++)
          signedProjection(profileTags(subjectA), createdAt: 2000 + i),
      ];
      expect(start(async), isEmpty);
      expect(proofCalls, 0);
      expect(session.unsubscribed, 1);
    });
  });

  test('a failed reconciliation is retried on the next refresh', () {
    fakeAsync((async) {
      setUpContainer();
      session.history = [signedProjection(profileTags(subjectA))];
      proofThrows = true;
      expect(start(async), isEmpty);
      expect(session.subscriptions, 1);

      proofThrows = false;
      async.elapse(const Duration(minutes: 5, seconds: 1));
      expect(session.subscriptions, 2);
      final profile = container.read(verifiedProfilesProvider)[subjectA];
      expect(profile?.displayName, 'Ada Lovelace');
      expect(profile?.checked, isTrue);
    });
  });

  test('an oversized event is dropped before buffering or proof', () {
    fakeAsync((async) {
      setUpContainer();
      final oversized = signedProjection(profileTags(subjectA));
      session.history = [
        NostrEvent(
          id: oversized.id,
          pubkey: oversized.pubkey,
          createdAt: oversized.createdAt,
          kind: oversized.kind,
          tags: oversized.tags,
          content: 'x' * 100000,
          sig: oversized.sig,
        ),
      ];
      expect(start(async), isEmpty);
      expect(provenEvents, 0);
      expect(session.unsubscribed, 0);
    });
  });

  test('live events are coalesced to the newest per subject', () {
    fakeAsync((async) {
      setUpContainer();
      expect(start(async), isEmpty);
      for (var i = 0; i < 50; i++) {
        session.emit!(
          signedProjection(profileTags(subjectA), createdAt: 3000 + i),
        );
      }
      async.elapse(const Duration(milliseconds: 100));
      expect(provenEvents, 1);
      expect(container.read(verifiedProfilesProvider)[subjectA], isNotNull);
    });
  });

  test('a replay keeps cached records until a fresh snapshot', () {
    fakeAsync((async) {
      setUpContainer();
      session.history = [signedProjection(profileTags(subjectA))];
      expect(start(async)[subjectA]?.checked, isTrue);
      expect(session.subscriptions, 1);

      // The relay retries; its replay resumes after the record, so it omits it.
      session.history = [];
      session.status!(RelaySubscriptionStatus.retrying);
      async.elapse(const Duration(milliseconds: 10));
      expect(
        container.read(verifiedProfilesProvider)[subjectA]?.checked,
        isFalse,
      );
      session.status!(RelaySubscriptionStatus.ready);
      async.elapse(const Duration(milliseconds: 100));
      expect(
        container.read(verifiedProfilesProvider)[subjectA]?.checked,
        isTrue,
      );

      // The next refresh resubscribes; the fresh snapshot proves absence.
      async.elapse(const Duration(minutes: 5, seconds: 1));
      expect(session.subscriptions, 2);
      expect(container.read(verifiedProfilesProvider), isEmpty);
    });
  });

  bool? checked(String subject) =>
      container.read(verifiedProfilesProvider)[subject]?.checked;

  /// Drops and restores the connection: cached records stay, and every batch
  /// is reopened on the new connection.
  void reconnect(FakeAsync async) {
    session.state = const SessionState(status: SessionStatus.disconnected);
    async.elapse(const Duration(milliseconds: 100));
    session.state = const SessionState(status: SessionStatus.connected);
    async.elapse(const Duration(milliseconds: 100));
  }

  test('a retry before the first EOSE does not prove absence', () {
    fakeAsync((async) {
      setUpContainer();
      final a = signedProjection(profileTags(subjectA), createdAt: 3000);
      final b = signedProjection(profileTags(subjectB), createdAt: 2000);
      session.history = [a, b];
      start(async, [subjectA, subjectB]);
      expect(checked(subjectA), isTrue);
      expect(checked(subjectB), isTrue);

      // On the next connection the relay sends A, closes before B, and the
      // retry's replay resumes after A, so it omits B.
      session.autoEose = false;
      reconnect(async);
      expect(checked(subjectB), isFalse);
      session.emit!(a);
      session.status!(RelaySubscriptionStatus.retrying);
      session.status!(RelaySubscriptionStatus.ready);
      async.elapse(const Duration(milliseconds: 100));
      expect(checked(subjectA), isTrue);
      // B is still cached, left for a fresh subscription to reconcile.
      expect(checked(subjectB), isFalse);

      session.autoEose = true;
      session.history = [a];
      async.elapse(const Duration(minutes: 5, seconds: 1));
      expect(checked(subjectA), isTrue);
      expect(container.read(verifiedProfilesProvider)[subjectB], isNull);
    });
  });

  test('a retry during proof leaves the batch unchecked', () {
    fakeAsync((async) {
      setUpContainer();
      session.autoEose = false;
      start(async);
      final gate = proofGate = Completer<void>();
      session.emit!(signedProjection(profileTags(subjectA)));
      session.status!(RelaySubscriptionStatus.ready);
      async.elapse(const Duration(milliseconds: 10));
      session.status!(RelaySubscriptionStatus.retrying);
      gate.complete();
      async.elapse(const Duration(milliseconds: 100));
      expect(checked(subjectA), isFalse);

      session.status!(RelaySubscriptionStatus.ready);
      async.elapse(const Duration(milliseconds: 100));
      expect(checked(subjectA), isFalse);
    });
  });

  test('a newer signed malformed projection clears the older one', () {
    fakeAsync((async) {
      setUpContainer();
      session.history = [signedProjection(profileTags(subjectA))];
      expect(start(async)[subjectA], isNotNull);

      session.emit!(
        signedProjection(
          profileTags(subjectA),
          createdAt: 3000,
          content: 'not empty',
        ),
      );
      async.elapse(const Duration(milliseconds: 100));
      expect(container.read(verifiedProfilesProvider), isEmpty);
    });
  });
}
