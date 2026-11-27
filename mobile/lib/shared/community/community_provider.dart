import '../hosted/known_community_sync.dart';
import '../hosted/known_community_sync_provider.dart';
import '../hosted/builderlab_account.dart';
import '../hosted/hosted_relay_auth.dart';

import 'dart:async';
import 'dart:developer' as developer;
import 'dart:math';

import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:flutter/foundation.dart' show visibleForTesting;
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../auth/auth_provider.dart';
import '../channels/channel_list_snapshot.dart';
import '../channels/saved_message_store.dart';
import '../diagnostics/diagnostic_reporter.dart';
import '../profile/profile_snapshot.dart';
import '../push/dev_push_lease.dart';
import '../push/push_bridge.dart';
import '../push/push_lease_revocation_outbox.dart';
import '../push/push_subscription.dart';
import '../relay/media_disk_cache.dart';
import '../relay/nostr_models.dart';
import '../relay/relay_session.dart';
import '../relay/signed_event_relay.dart';
import '../relay/relay_provider.dart'
    show RelayConfig, pubkeyFromNsec, relayConfigProvider;
import 'community.dart';
import 'community_storage.dart';

final class CommunityTransitionCoordinator {
  final Map<Object, Future<void> Function()> _callbacks = {};
  Future<void>? _inFlight;
  Future<void> _operationTail = Future.value();

  void Function() register(Future<void> Function() callback) {
    final owner = Object();
    _callbacks[owner] = callback;
    return () => _callbacks.remove(owner);
  }

  /// Runs a complete community mutation after every earlier mutation finishes.
  ///
  /// Cleanup alone is intentionally memoized by [run], but callers that also
  /// mutate credentials or the active community must queue the whole operation
  /// here so they cannot race after sharing the same cleanup future.
  Future<void> runExclusive(Future<void> Function() operation) {
    final result = _operationTail.then((_) => operation());
    _operationTail = result.then<void>(
      (_) {},
      onError: (Object _, StackTrace _) {},
    );
    return result;
  }

  Future<void> run() {
    final inFlight = _inFlight;
    if (inFlight != null) return inFlight;

    final run = _runCallbacks();
    _inFlight = run;
    run.whenComplete(() {
      if (identical(_inFlight, run)) _inFlight = null;
    });
    return run;
  }

  Future<void> _runCallbacks() async {
    await Future.wait(
      _callbacks.values.map((callback) async {
        try {
          await callback();
        } catch (error, stackTrace) {
          developer.log(
            'Community transition cleanup failed',
            name: 'buzz.community',
            error: error,
            stackTrace: stackTrace,
          );
        }
      }),
    );
  }
}

final communityTransitionProvider = Provider<CommunityTransitionCoordinator>(
  (ref) => CommunityTransitionCoordinator(),
);

final communityStorageProvider = Provider<CommunityStorage>((ref) {
  return CommunityStorage(diagnostics: ref.read(diagnosticReporterProvider));
});

typedef CommunitySnapshotWriter =
    Future<void> Function(List<Community> communities);

/// Exports persisted communities to the notification service extension.
final communitySnapshotWriterProvider = Provider<CommunitySnapshotWriter>((
  ref,
) {
  return registerBuzzPushCommunitySnapshot;
});

final _communitySnapshotSyncProvider = Provider<_CommunitySnapshotSync>((ref) {
  return _CommunitySnapshotSync(ref.read(communitySnapshotWriterProvider));
});

/// Publishes a community's inactive push lease. With a [deadline], the
/// attempt fails once it passes, and an EVENT not yet sent by then never is.
typedef CommunityPushLeaseDeactivator =
    Future<void> Function(
      Community community, {
      int? generation,
      DateTime? deadline,
    });

final communityPushLeaseDeactivatorProvider =
    Provider<CommunityPushLeaseDeactivator>((ref) {
      return (community, {generation, deadline}) =>
          _deactivateCommunityPushLease(
            community,
            generation: generation,
            deadline: deadline,
            relayAuth: ref.read(hostedRelayAuthProvider),
            diagnostics: ref.read(diagnosticReporterProvider),
          );
    });

/// Reads the push lease the active community's relay holds for this
/// installation, or null when it holds none. Throws when the relay doesn't
/// answer, or when the active community changes while the read is out.
typedef CommunityPushLeaseReader =
    Future<BuzzPushLeaseState?> Function(Community community);

final communityPushLeaseReaderProvider = Provider<CommunityPushLeaseReader>((
  ref,
) {
  return (community) => readCommunityPushLease(
    community,
    query: ref.read(relaySessionProvider.notifier).queryRelay,
    currentRelay: () => ref.read(relayConfigProvider),
  );
});

typedef CommunityPushLeaseRevocationEnqueuer =
    Future<bool> Function(Community community);

final communityPushLeaseRevocationEnqueuerProvider =
    Provider<CommunityPushLeaseRevocationEnqueuer>((ref) {
      return ref.watch(buzzPushLeaseRevocationOutboxProvider).enqueueCommunity;
    });

typedef CommunityPushLeaseRevocationTrigger = Future<void> Function();

final communityPushLeaseRevocationTriggerProvider =
    Provider<CommunityPushLeaseRevocationTrigger>((ref) {
      return ref.watch(buzzPushLeaseRevocationOutboxProvider).trigger;
    });

/// The relay key, member and installation a community's push lease is for.
typedef _PushLeaseTarget = ({
  BuzzPushLeaseDescriptor descriptor,
  String nsec,
  String memberPubkey,
  String installationId,
});

Future<_PushLeaseTarget> _pushLeaseTarget(
  Community community, {
  DateTime? deadline,
}) async {
  final nsec = community.nsec;
  if (nsec == null || nsec.isEmpty) {
    throw StateError('Push lease requires community signing key.');
  }
  final decoded = nostr.Nip19.decode(payload: nsec);
  final memberPubkey = community.pubkey ?? derivePublicKeyHex(decoded.data);
  final descriptor = await fetchBuzzPushLeaseDescriptor(
    canonicalBuzzPushRelayHttpUrl(community.relayUrl),
    timeout: _within(deadline, const Duration(seconds: 8)),
  );
  _checkDeadline(deadline);
  // A gateway change leaves no grant for the relay, but the lease the relay
  // holds is still addressed by the id the community saved.
  var installationId = community.pushLeaseInstallationId;
  if (installationId == null) {
    final matchingGrant = (await readBuzzPushEndpointGrants())
        .where((grant) => grant.relayOrigin == descriptor.origin)
        .firstOrNull;
    _checkDeadline(deadline);
    if (matchingGrant == null) {
      throw StateError('No endpoint grant exists for the push lease.');
    }
    installationId = matchingGrant.installationId;
  }
  return (
    descriptor: descriptor,
    nsec: nsec,
    memberPubkey: memberPubkey,
    installationId: installationId,
  );
}

/// [limit], or less when [deadline] comes sooner.
Duration _within(DateTime? deadline, Duration limit) {
  if (deadline == null) return limit;
  final remaining = deadline.difference(DateTime.now());
  if (remaining.isNegative) return Duration.zero;
  return remaining < limit ? remaining : limit;
}

void _checkDeadline(DateTime? deadline) {
  if (deadline != null && !DateTime.now().isBefore(deadline)) {
    throw TimeoutException('The push opt-out ran out of time');
  }
}

/// Reads [community]'s push lease through [query], which sends to whichever
/// relay [currentRelay] names when it runs. Throws when that relay or key is
/// no longer [community]'s: a switch while the read waits would otherwise
/// settle one community from another's relay.
@visibleForTesting
Future<BuzzPushLeaseState?> readCommunityPushLease(
  Community community, {
  required Future<List<NostrEvent>> Function(List<NostrFilter> filters) query,
  required RelayConfig Function() currentRelay,
}) async {
  final expected = RelayConfig(baseUrl: community.relayUrl).baseUrl;
  void checkStillCurrent() {
    final relay = currentRelay();
    if (relay.baseUrl != expected || relay.nsec != community.nsec) {
      throw StateError('The active community changed during the lease read.');
    }
  }

  final target = await _pushLeaseTarget(community);
  checkStillCurrent();
  final events = await query([
    NostrFilter(
      kinds: const [buzzPushLeaseKind],
      authors: [target.memberPubkey],
      tags: {
        '#d': [target.installationId],
      },
      limit: 1,
      // A replica can lag the write that replaced the lease.
      extensions: const {'consistency': 'strong'},
    ),
  ]);
  checkStillCurrent();
  final event = events.firstOrNull;
  if (event == null) return null;
  return decryptBuzzPushLease(
    event: event,
    descriptor: target.descriptor,
    nsec: target.nsec,
  );
}

Future<void> _deactivateCommunityPushLease(
  Community community, {
  int? generation,
  DateTime? deadline,
  required HostedRelayAuth relayAuth,
  required DiagnosticReporter diagnostics,
}) async {
  final state = community.pushSubscriptionState;
  final acceptedGeneration = state.acceptedGeneration;
  if (acceptedGeneration == null && generation == null) {
    return;
  }
  final (:descriptor, :nsec, :memberPubkey, :installationId) =
      await _pushLeaseTarget(community, deadline: deadline);
  final uri = Uri.parse(community.relayUrl);
  final httpScheme = switch (uri.scheme) {
    'wss' => 'https',
    'ws' => 'http',
    _ => uri.scheme,
  };
  final wsScheme = httpScheme == 'https' ? 'wss' : 'ws';
  final wsUrl = uri.replace(scheme: wsScheme).toString();
  // The community being removed may not be the active session, so ask for its
  // own assertion: a hosted relay refuses the socket upgrade without one.
  final relayConfig = RelayConfig(
    baseUrl: uri.replace(scheme: httpScheme).toString(),
    nsec: nsec,
  );
  // Skip over the one renewal generation that could already be in flight
  // when removal begins. Strict relay monotonicity then makes any stale
  // active publication lose to this tombstone.
  final tombstoneGeneration =
      generation ?? (state.generationCursor ?? acceptedGeneration!) + 2;
  await publishBuzzPushLeaseTombstone(
    descriptor: descriptor,
    installationId: installationId,
    generation: tombstoneGeneration,
    nsec: nsec,
    memberPubkey: memberPubkey,
    submit:
        ({required kind, required content, required tags, createdAt}) async {
          final headers = await relayAuth.headers(
            relayConfig,
            background: true,
            fresh: true,
          );
          _checkDeadline(deadline);
          return submitSignedEventOnce(
            wsUrl: wsUrl,
            headers: headers,
            nsec: nsec,
            kind: kind,
            content: content,
            tags: tags,
            createdAt: createdAt,
            deadline: deadline,
            diagnostics: diagnostics,
          );
        },
  );
  pushLeaseCleanupError.value = null;
}

class _CommunitySnapshotSync {
  _CommunitySnapshotSync(this._writer);

  final CommunitySnapshotWriter _writer;
  String? _lastSuccessfulSnapshot;
  Future<void> _mutationTail = Future.value();

  Future<void> _serializeMutation(Future<void> Function() operation) {
    final result = _mutationTail.then((_) => operation());
    _mutationTail = result.then<void>(
      (_) {},
      onError: (Object _, StackTrace _) {},
    );
    return result;
  }

  Future<void> write(List<Community> communities) =>
      _serializeMutation(() => _write(communities));

  Future<void> _write(List<Community> communities) async {
    final effectiveCommunities = communities;
    final contentFingerprint = effectiveCommunities
        .map(
          (community) => [
            community.id,
            community.name,
            community.relayUrl,
            community.pubkey,
            community.nsec,
            community.pushNotificationsEnabled,
            buzzPushSubscriptionStateFingerprint(
              community.pushSubscriptionState,
            ),
          ].join('\u0000'),
        )
        .join('\u0001');
    if (contentFingerprint == _lastSuccessfulSnapshot) return;
    await _writer(effectiveCommunities);
    _lastSuccessfulSnapshot = contentFingerprint;
  }
}

Future<void> syncCommunitySnapshot(Ref ref, List<Community> communities) async {
  try {
    await ref.read(_communitySnapshotSyncProvider).write(communities);
    pushCommunitySnapshotError.value = null;
  } catch (error, stackTrace) {
    reportPushCommunitySnapshotError(error, stackTrace);
  }
}

Future<void> syncStoredCommunitySnapshot(Ref ref) async {
  final communities = await ref.read(communityStorageProvider).loadAll();
  await syncCommunitySnapshot(ref, communities);
}

/// How long an opt-out may stay pending, counted from the tap, before it
/// fails (docs/mobile-push-suppression.md, rule 29).
const pushOptOutTimeout = Duration(seconds: 10);

/// Lets the deactivator's own deadline, which knows whether the tombstone was
/// sent, fire before the backstop.
const _pushOptOutBackstopGrace = Duration(milliseconds: 500);

/// Where an opt-out started in this app run stands.
enum PushOptOutStatus {
  /// Its tombstone is being published. Settings shows it pending.
  turningOff,

  /// It failed or went unconfirmed. Settings shows the relay's last confirmed
  /// setting and warns that notifications may continue.
  failed,
}

/// The opt-out status of each community, by id. A community with none either
/// has no opt-out under way, or has one left unconfirmed by an earlier run.
class PushOptOutStatusNotifier extends Notifier<Map<String, PushOptOutStatus>> {
  @override
  Map<String, PushOptOutStatus> build() => const {};

  bool isTurningOff(String id) => state[id] == PushOptOutStatus.turningOff;

  void set(String id, PushOptOutStatus status) =>
      state = {...state, id: status};

  void clear(String id) {
    if (!state.containsKey(id)) return;
    state = {...state}..remove(id);
  }
}

final pushOptOutStatusProvider =
    NotifierProvider<PushOptOutStatusNotifier, Map<String, PushOptOutStatus>>(
      PushOptOutStatusNotifier.new,
    );

class CommunityListNotifier extends AsyncNotifier<List<Community>> {
  Future<void> _pushMutationTail = Future.value();

  Future<T> _serializePushMutation<T>(Future<T> Function() operation) {
    final result = _pushMutationTail.then((_) => operation());
    _pushMutationTail = result.then<void>(
      (_) {},
      onError: (Object _, StackTrace _) {},
    );
    return result;
  }

  @override
  Future<List<Community>> build() async {
    final storage = ref.read(communityStorageProvider);
    final communities = await storage.loadAll();
    unawaited(_removeOrphanChannelListSnapshots(communities));
    unawaited(_removeOrphanMediaCaches(fallback: communities));
    await syncCommunitySnapshot(ref, communities);
    return communities;
  }

  /// Removes a saved identity without letting awaited cleanup change the account
  /// that confirmed its remote edit. An explicit null confirmation is local-only.
  Future<void> removeCommunity(
    String id, {
    String? confirmedSubject,
    bool accountConfirmed = false,
  }) async {
    final account = ref.read(builderlabAccountProvider);
    final sync = ref.read(knownCommunitySyncProvider);
    final subject = sync == null
        ? null
        : accountConfirmed
        ? confirmedSubject
        : await account.localEditSubject();
    return ref.read(communityTransitionProvider).runExclusive(() async {
      var revocationJournaled = false;
      final storage = ref.read(communityStorageProvider);
      final activeId = await storage.loadActiveId();
      if (activeId == id) {
        await ref.read(communityTransitionProvider).run();
      }
      final current = state.value ?? await storage.loadAll();
      final removedIndex = current.indexWhere(
        (community) => community.id == id,
      );
      if (sync != null && await account.localEditSubject() != subject) {
        throw AccountFailure.differentAccount;
      }
      if (removedIndex >= 0) {
        await _removeChannelListSnapshot(current[removedIndex]);
        // Persist every remote-cleanup dependency before erasing credentials.
        // This local transaction is the only removal prerequisite. Relay I/O
        // starts after the community and NSE snapshot have been removed.
        revocationJournaled = await ref.read(
          communityPushLeaseRevocationEnqueuerProvider,
        )(current[removedIndex]);
      }
      // A durable revocation commits this removal to its confirmed account.
      // Finish local deletion even if the account changed during journaling;
      // aborting now would leave a saved community with a revoked push lease.
      if (!revocationJournaled &&
          sync != null &&
          await account.localEditSubject() != subject) {
        throw AccountFailure.differentAccount;
      }
      await storage.remove(
        id,
        accountSyncOwner: subject == null
            ? null
            : '${account.accountOrigin.origin}\n$subject',
      );
      ref.read(knownCommunitySyncProvider)?.trigger();

      final updatedList = current.where((w) => w.id != id).toList();
      state = AsyncData(updatedList);
      await _removeOrphanChannelListSnapshots(updatedList);
      await syncCommunitySnapshot(ref, updatedList);

      // If we removed the active community, switch to another or sign out.
      if (activeId == id) {
        final remaining = state.value ?? [];
        if (remaining.isNotEmpty) {
          await storage.saveActiveId(remaining.first.id);
          _reportAs(remaining.first);
          // Reassign list state so activeCommunityProvider picks up the new ID.
          state = AsyncData([...remaining]);
          ref.invalidate(authProvider);
        } else {
          await storage.clearActiveId();
          _reportAs(null);
          // Invalidate auth so it re-evaluates against the now-empty storage
          // and transitions to unauthenticated.
          ref.invalidate(authProvider);
        }
      }
      // Deleting up to a full cache can take a while; never hold the removal
      // on it. The launch sweep retries a failure.
      unawaited(_removeOrphanMediaCaches());
      if (revocationJournaled) {
        unawaited(
          ref.read(communityPushLeaseRevocationTriggerProvider)().catchError((
            Object error,
            StackTrace stackTrace,
          ) {
            reportPushLeaseCleanupError(error, stackTrace);
          }),
        );
      }
    });
  }

  String? _channelSnapshotPubkey(Community community) =>
      pubkeyFromNsec(community.nsec?.trim()) ?? community.pubkey;

  Future<void> _removeChannelListSnapshot(Community community) async {
    final snapshots = ref.read(channelListSnapshotStorageProvider);
    final profiles = ref.read(profileSnapshotStorageProvider);
    final pubkey = _channelSnapshotPubkey(community);
    if (pubkey == null) return;
    try {
      // Retire writers before the revocation journal or credential changes.
      // A disposable cache must never prevent local sign-out.
      await Future.wait([
        ?snapshots?.remove(pubkey, community.relayUrl),
        ?profiles?.remove(pubkey, community.relayUrl),
        ref.read(savedMessageStoreProvider).removeScope((
          communityId: community.id,
          pubkey: pubkey,
        )),
      ]);
    } catch (error, stackTrace) {
      developer.log(
        'Channel snapshot deletion failed; orphan cleanup will retry',
        name: 'buzz.community',
        error: error,
        stackTrace: stackTrace,
      );
    }
  }

  Future<void> _removeOrphanChannelListSnapshots(
    List<Community> communities,
  ) async {
    final snapshots = ref.read(channelListSnapshotStorageProvider);
    final profiles = ref.read(profileSnapshotStorageProvider);
    final scopes = [
      for (final community in communities)
        if (_channelSnapshotPubkey(community) case final pubkey?)
          (pubkey: pubkey, relay: community.relayUrl),
    ];
    await Future.wait([
      ?snapshots?.removeOrphans({
        for (final scope in scopes)
          ChannelListSnapshotStorage.key(scope.pubkey, scope.relay),
      }),
      ?profiles?.removeOrphans({
        for (final scope in scopes)
          ProfileSnapshotStorage.key(scope.pubkey, scope.relay),
      }),
      ref.read(savedMessageStoreProvider).removeOrphans({
        for (final community in communities)
          if (_channelSnapshotPubkey(community) case final pubkey?)
            (communityId: community.id, pubkey: pubkey),
      }),
    ]);
  }

  /// Deletes the image caches and saved channel directories of communities no
  /// longer in the list. A failure, or a write that lands after the delete, is
  /// swept on the next launch.
  ///
  /// Sweeps run one at a time, each keeping what the community list holds
  /// when it starts, so an older sweep can never delete a newer community's
  /// cache. [fallback] stands in for the list while it is still loading.
  Future<void> _removeOrphanMediaCaches({List<Community>? fallback}) =>
      _mediaSweep = _mediaSweep.then((_) => _sweepMediaCaches(fallback));

  Future<void> _mediaSweep = Future.value();

  Future<void> _sweepMediaCaches(List<Community>? fallback) async {
    try {
      final root = await ref.read(mediaCacheRootProvider.future);
      if (root == null) return;
      List<Community>? current() =>
          state.isLoading ? fallback ?? state.value : state.value;
      var communities = current();
      if (communities == null) return;
      Set<String> retained() {
        // A list briefly unavailable mid-sweep keeps the last one seen.
        communities = current() ?? communities;
        return {
          for (final community in communities!)
            if (_channelSnapshotPubkey(community) != null) community.id,
        };
      }

      await MediaDiskCache.removeOrphans(root, retained);
    } catch (error, stackTrace) {
      developer.log(
        'Media cache deletion failed; the next launch will retry',
        name: 'buzz.community',
        error: error,
        stackTrace: stackTrace,
      );
    }
  }

  /// Names [community]'s member on reports from now on. The providers that
  /// derive the member from the active community settle a frame later, and a
  /// report raised in between would name the previous member.
  void _reportAs(Community? community) => ref
      .read(diagnosticReporterProvider)
      .setMember(pubkeyFromNsec(community?.nsec));

  Future<void> switchCommunity(String id) {
    return ref.read(communityTransitionProvider).runExclusive(() async {
      final storage = ref.read(communityStorageProvider);
      final activeId = await storage.loadActiveId();
      if (activeId == id) return;
      // From storage: while the list reloads (after sign-in, say), state can
      // still hold the previous list. Read before anything changes, so a
      // failed read leaves the switch to retry.
      final target = (await storage.loadAll())
          .where((c) => c.id == id)
          .firstOrNull;
      await ref.read(communityTransitionProvider).run();
      await storage.saveActiveId(id);
      _reportAs(target);
      // Reassign list state to trigger activeCommunityProvider (which watches
      // communityListProvider.future) to rebuild and pick up the new active ID.
      // We can't use ref.invalidate(activeCommunityProvider) here because that
      // creates a circular dependency — activeCommunityProvider watches us.
      state = AsyncData([...state.value ?? []]);
      // Invalidate auth so AuthState.community reflects the new active community.
      ref.invalidate(authProvider);
    });
  }

  Future<void> updateDesiredPushSubscriptions(
    String id,
    List<BuzzPushSubscription> desired,
  ) => _serializePushMutation(() async {
    final storage = ref.read(communityStorageProvider);
    final current = state.value ?? await storage.loadAll();
    final index = current.indexWhere((community) => community.id == id);
    if (index < 0) return;

    final community = current[index];
    if (buzzPushSubscriptionsFingerprint(
          community.pushSubscriptionState.desired,
        ) ==
        buzzPushSubscriptionsFingerprint(desired)) {
      return;
    }
    final updated = community.copyWith(
      pushSubscriptionState: community.pushSubscriptionState.withDesired(
        desired,
      ),
    );
    await storage.save(updated);
    final updatedList = [...current]..[index] = updated;
    state = AsyncData(updatedList);
    await syncCommunitySnapshot(ref, updatedList);
  });

  Future<int> reservePushLeaseGeneration(String id) {
    return _serializePushMutation(() async {
      final storage = ref.read(communityStorageProvider);
      final current = state.value ?? await storage.loadAll();
      final index = current.indexWhere((community) => community.id == id);
      if (index < 0) throw StateError('Push community is unavailable.');

      final community = current[index];
      if (!community.pushNotificationsEnabled) {
        throw StateError('Push notifications are disabled.');
      }
      final cursor =
          community.pushSubscriptionState.generationCursor ??
          community.pushSubscriptionState.acceptedGeneration ??
          0;
      final generation = cursor + 1;
      final updated = community.copyWith(
        pushSubscriptionState: community.pushSubscriptionState
            .withReservedGeneration(generation),
      );
      await storage.save(updated);
      final updatedList = [...current]..[index] = updated;
      state = AsyncData(updatedList);
      await syncCommunitySnapshot(ref, updatedList);
      return generation;
    });
  }

  Future<bool> markPushLeaseAccepted(
    String id, {
    required List<BuzzPushSubscription> subscriptions,
    required int generation,
  }) => _serializePushMutation(() async {
    final storage = ref.read(communityStorageProvider);
    final current = state.value ?? await storage.loadAll();
    final index = current.indexWhere((community) => community.id == id);
    if (index < 0) return false;

    final community = current[index];
    final acceptedGeneration =
        community.pushSubscriptionState.acceptedGeneration ?? 0;
    final generationCursor =
        community.pushSubscriptionState.generationCursor ?? 0;
    if (generation < max(acceptedGeneration, generationCursor)) return false;
    if (buzzPushSubscriptionsFingerprint(
          community.pushSubscriptionState.desired,
        ) !=
        buzzPushSubscriptionsFingerprint(subscriptions)) {
      return false;
    }
    final updated = community.copyWith(
      pushSubscriptionState: community.pushSubscriptionState.withAccepted(
        subscriptions: subscriptions,
        generation: generation,
      ),
    );
    await storage.save(updated);
    final updatedList = [...current]..[index] = updated;
    state = AsyncData(updatedList);
    await syncCommunitySnapshot(ref, updatedList);
    return true;
  });

  Future<void> setPushNotificationsEnabled(String id, bool enabled) async {
    final statuses = ref.read(pushOptOutStatusProvider.notifier);
    if (enabled) {
      statuses.clear(id);
      await _setPushNotificationsEnabled(id, enabled: true);
      return;
    }
    if (statuses.isTurningOff(id)) return;
    // The deadline runs from the tap, across every step of the attempt.
    final deadline = DateTime.now().add(pushOptOutTimeout);
    // Set before the opt-out is saved, so Settings shows it pending from the
    // first frame where push is off.
    statuses.set(id, PushOptOutStatus.turningOff);
    var journaled = false;
    try {
      journaled = await _setPushNotificationsEnabled(id, enabled: false);
    } finally {
      if (!journaled) statuses.clear(id);
    }
    if (journaled) {
      await _publishOptOut(id, deadline: deadline, advanceGeneration: false);
    }
  }

  /// Saves the switch. Returns whether an opt-out tombstone is now pending.
  Future<bool> _setPushNotificationsEnabled(
    String id, {
    required bool enabled,
  }) async {
    var shouldDeactivate = false;
    await _serializePushMutation(() async {
      final storage = ref.read(communityStorageProvider);
      final current = state.value ?? await storage.loadAll();
      final index = current.indexWhere((community) => community.id == id);
      if (index < 0) return;

      final community = current[index];
      if (community.pushNotificationsEnabled == enabled) return;
      var pushState = community.pushSubscriptionState;
      if (!enabled &&
          (pushState.acceptedGeneration != null ||
              pushState.generationCursor != null)) {
        final cursor =
            pushState.generationCursor ?? pushState.acceptedGeneration ?? 0;
        pushState = pushState.withPendingTombstone(cursor + 1);
      }
      final updated = community.copyWith(
        pushNotificationsEnabled: enabled,
        pushSubscriptionState: pushState,
      );
      await storage.save(updated);
      final updatedList = [...current]..[index] = updated;
      state = AsyncData(updatedList);
      await syncCommunitySnapshot(ref, updatedList);
      shouldDeactivate =
          !enabled && pushState.pendingTombstoneGeneration != null;
    });
    return shouldDeactivate;
  }

  /// Makes one explicit attempt to finish a failed opt-out, for the user's
  /// Retry. Nothing retries an opt-out on its own.
  Future<void> retryPushOptOut(String id) async {
    final community = (state.value ?? const <Community>[])
        .where((community) => community.id == id)
        .firstOrNull;
    if (community == null) return;
    // A failure the relay never saw restored push on; turn it off afresh.
    if (community.pushNotificationsEnabled) {
      return setPushNotificationsEnabled(id, false);
    }
    final statuses = ref.read(pushOptOutStatusProvider.notifier);
    if (statuses.isTurningOff(id)) return;
    final deadline = DateTime.now().add(pushOptOutTimeout);
    statuses.set(id, PushOptOutStatus.turningOff);
    await _publishOptOut(id, deadline: deadline, advanceGeneration: true);
  }

  /// Publishes the saved opt-out tombstone and records how it ended. A
  /// failure is never retried automatically.
  Future<void> _publishOptOut(
    String id, {
    required DateTime deadline,
    required bool advanceGeneration,
  }) async {
    final statuses = ref.read(pushOptOutStatusProvider.notifier);
    final int? accepted;
    try {
      accepted = await _publishPendingTombstone(
        id,
        deadline: deadline,
        advanceGeneration: advanceGeneration,
      );
    } catch (error, stackTrace) {
      reportPushLeaseCleanupError(error, stackTrace);
      try {
        await _settleFailedOptOut(
          id,
          // A Retry only runs on an opt-out an earlier attempt left pending,
          // which that attempt's relay may hold. Its own failure, even before
          // sending, can't show the relay never received an opt-out.
          maybeReceived:
              advanceGeneration || error is SignedEventUnacknowledged,
        );
      } finally {
        statuses.set(id, PushOptOutStatus.failed);
      }
      return;
    }
    if (accepted != null) {
      try {
        await _markPushLeaseTombstoneAccepted(id, accepted);
      } catch (error, stackTrace) {
        // The relay accepted the tombstone; only recording that failed. The
        // opt-out stays pending, never reversed, until a lease read settles it.
        reportPushLeaseCleanupError(error, stackTrace);
        statuses.set(id, PushOptOutStatus.failed);
        return;
      }
    }
    pushLeaseCleanupError.value = null;
    statuses.clear(id);
  }

  /// Publishes a durably journaled opt-out tombstone. Returns the generation
  /// the relay accepted, which the caller records, or null when no opt-out
  /// was pending.
  ///
  /// A retry advances the generation before network I/O. That makes an
  /// ambiguous relay-commit/local-save failure idempotent in effect: the next
  /// inactive replacement wins even if the prior tombstone already committed.
  Future<int?> _publishPendingTombstone(
    String id, {
    required DateTime deadline,
    required bool advanceGeneration,
  }) async {
    Community? pendingCommunity;
    int? pendingGeneration;
    await _serializePushMutation(() async {
      final storage = ref.read(communityStorageProvider);
      final current = state.value ?? await storage.loadAll();
      final index = current.indexWhere((community) => community.id == id);
      if (index < 0) return;
      final community = current[index];
      var pushState = community.pushSubscriptionState;
      final pending = pushState.pendingTombstoneGeneration;
      if (community.pushNotificationsEnabled || pending == null) return;
      if (advanceGeneration) {
        final cursor = pushState.generationCursor ?? pending;
        pushState = pushState.withPendingTombstone(cursor + 1);
      }
      final updated = community.copyWith(pushSubscriptionState: pushState);
      if (!identical(pushState, community.pushSubscriptionState)) {
        await storage.save(updated);
        final updatedList = [...current]..[index] = updated;
        state = AsyncData(updatedList);
        await syncCommunitySnapshot(ref, updatedList);
      }
      pendingCommunity = updated;
      pendingGeneration = pushState.pendingTombstoneGeneration;
    });
    final community = pendingCommunity;
    final generation = pendingGeneration;
    if (community == null || generation == null) return null;
    // The deactivator honors the deadline itself and says whether the relay
    // could have seen the tombstone. This backstop only catches a step that
    // hangs past it, and can't tell, so it keeps the opt-out unconfirmed.
    await ref
        .read(communityPushLeaseDeactivatorProvider)(
          community,
          generation: generation,
          deadline: deadline,
        )
        .timeout(
          _within(deadline, pushOptOutTimeout) + _pushOptOutBackstopGrace,
          onTimeout: () => throw SignedEventUnacknowledged(
            TimeoutException('The push opt-out ran out of time'),
          ),
        );
    return generation;
  }

  /// Shows the relay's last confirmed setting after a failed opt-out.
  ///
  /// When the relay never received the tombstone and its confirmed lease is
  /// active, push is on again for real. Otherwise the relay may already be off
  /// ([maybeReceived]), or its confirmed state is off, so the pending
  /// tombstone stays: nothing republishes an active lease until a lease read
  /// ([reconcilePushOptOut]) settles it.
  Future<void> _settleFailedOptOut(String id, {required bool maybeReceived}) =>
      _serializePushMutation(() async {
        if (maybeReceived) return;
        final storage = ref.read(communityStorageProvider);
        final current = state.value ?? await storage.loadAll();
        final index = current.indexWhere((community) => community.id == id);
        if (index < 0) return;
        final community = current[index];
        final pushState = community.pushSubscriptionState;
        if (community.pushNotificationsEnabled ||
            pushState.pendingTombstoneGeneration == null ||
            pushState.accepted == null) {
          return;
        }
        final updated = community.copyWith(
          pushNotificationsEnabled: true,
          pushSubscriptionState: pushState.withoutPendingTombstone(),
        );
        await storage.save(updated);
        final updatedList = [...current]..[index] = updated;
        state = AsyncData(updatedList);
        await syncCommunitySnapshot(ref, updatedList);
      });

  /// Settles an unconfirmed opt-out from the lease the relay actually holds,
  /// on the next successful sync. An inactive lease, or none, confirms push
  /// is off. An active one turns push back on and keeps the warning. Throws
  /// when the relay doesn't answer, leaving the opt-out unconfirmed until the
  /// next read.
  Future<void> reconcilePushOptOut(String id) async {
    final statuses = ref.read(pushOptOutStatusProvider.notifier);
    bool unsettled(Community? community) =>
        community != null &&
        !community.pushNotificationsEnabled &&
        community.pushSubscriptionState.pendingTombstoneGeneration != null;
    final community = (state.value ?? const <Community>[])
        .where((community) => community.id == id)
        .firstOrNull;
    if (statuses.isTurningOff(id) || !unsettled(community)) return;
    final generation =
        community!.pushSubscriptionState.pendingTombstoneGeneration;
    final lease = await ref.read(communityPushLeaseReaderProvider)(community);
    final active = lease?.active ?? false;
    final settled = await _serializePushMutation(() async {
      // A Retry may have started while the read was out. Even once it has
      // ended, the read predates its tombstone, so it can't settle it.
      if (statuses.isTurningOff(id)) return false;
      final storage = ref.read(communityStorageProvider);
      final current = state.value ?? await storage.loadAll();
      final index = current.indexWhere((community) => community.id == id);
      if (index < 0 || !unsettled(current[index])) return false;
      if (current[index].pushSubscriptionState.pendingTombstoneGeneration !=
          generation) {
        return false;
      }
      final community = current[index];
      final pushState = community.pushSubscriptionState;
      final updated = active
          ? community.copyWith(
              pushNotificationsEnabled: true,
              pushSubscriptionState: pushState.withoutPendingTombstone(),
            )
          : community.copyWith(
              pushSubscriptionState: pushState.withAcceptedTombstone(
                pushState.pendingTombstoneGeneration!,
              ),
            );
      await storage.save(updated);
      final updatedList = [...current]..[index] = updated;
      state = AsyncData(updatedList);
      await syncCommunitySnapshot(ref, updatedList);
      return true;
    });
    if (!settled) return;
    if (active) {
      statuses.set(id, PushOptOutStatus.failed);
    } else {
      statuses.clear(id);
    }
  }

  Future<void> _markPushLeaseTombstoneAccepted(String id, int generation) =>
      _serializePushMutation(() async {
        final storage = ref.read(communityStorageProvider);
        final current = state.value ?? await storage.loadAll();
        final index = current.indexWhere((community) => community.id == id);
        if (index < 0) return;
        final community = current[index];
        final updatedState = community.pushSubscriptionState
            .withAcceptedTombstone(generation);
        if (identical(updatedState, community.pushSubscriptionState)) return;
        final updated = community.copyWith(pushSubscriptionState: updatedState);
        await storage.save(updated);
        final updatedList = [...current]..[index] = updated;
        state = AsyncData(updatedList);
        await syncCommunitySnapshot(ref, updatedList);
      });
}

final communityListProvider =
    AsyncNotifierProvider<CommunityListNotifier, List<Community>>(
      CommunityListNotifier.new,
    );

/// The currently active community, derived from the stored active ID and
/// the community list.
final activeCommunityProvider = FutureProvider<Community?>((ref) async {
  final communities = await ref.watch(communityListProvider.future);
  final storage = ref.read(communityStorageProvider);
  final activeId = await storage.loadActiveId();

  if (communities.isEmpty) return null;

  // This projection can outlive the list it captured above. Never persist a
  // fallback here: a newer authentication may already have selected a community
  // absent from that list. Authentication owns durable active-ID repair.
  return communities.firstWhere(
    (community) => community.id == activeId,
    orElse: () => communities.first,
  );
});
