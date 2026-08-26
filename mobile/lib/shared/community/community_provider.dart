import 'dart:developer' as developer;
import 'dart:math';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../auth/auth_provider.dart';
import '../push/dev_push_lease.dart';
import '../push/push_bridge.dart';
import '../push/push_subscription.dart';
import '../relay/signed_event_relay.dart';
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
  return CommunityStorage();
});

typedef CommunitySnapshotWriter =
    Future<void> Function(List<Community> communities);

/// Writes the complete persisted community set to storage shared with the iOS
/// notification service extension. Tests override this provider to verify that
/// every persistence path refreshes (or clears) the native snapshot.
final communitySnapshotWriterProvider = Provider<CommunitySnapshotWriter>((
  ref,
) {
  return registerBuzzPushCommunitySnapshot;
});

final _communitySnapshotSyncProvider = Provider<_CommunitySnapshotSync>((ref) {
  return _CommunitySnapshotSync(ref.read(communitySnapshotWriterProvider));
});

typedef CommunityPushLeaseDeactivator =
    Future<void> Function(Community community, {int? generation});

final communityPushLeaseDeactivatorProvider =
    Provider<CommunityPushLeaseDeactivator>((ref) {
      return (community, {generation}) =>
          _deactivateCommunityPushLease(community, generation: generation);
    });

Future<void> _deactivateCommunityPushLease(
  Community community, {
  int? generation,
}) async {
  final state = community.pushSubscriptionState;
  final acceptedGeneration = state.acceptedGeneration;
  final nsec = community.nsec;
  if ((acceptedGeneration == null && generation == null) ||
      nsec == null ||
      nsec.isEmpty) {
    return;
  }
  try {
    final decoded = nostr.Nip19.decode(payload: nsec);
    final memberPubkey = community.pubkey ?? nostr.Keys(decoded.data).public;
    final descriptor = await fetchBuzzPushLeaseDescriptor(community.relayUrl);
    final installationId = (await readBuzzPushEndpointGrants())
        .where(
          (grant) =>
              grant.relayOrigin == descriptor.origin &&
              grant.appProfile == buzzDevPushAppProfile,
        )
        .map((grant) => grant.installationId)
        .firstOrNull;
    if (installationId == null) return;
    final uri = Uri.parse(community.relayUrl);
    final httpScheme = switch (uri.scheme) {
      'wss' => 'https',
      'ws' => 'http',
      _ => uri.scheme,
    };
    final wsScheme = httpScheme == 'https' ? 'wss' : 'ws';
    final wsUrl = uri.replace(scheme: wsScheme).toString();
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
      submit: ({required kind, required content, required tags, createdAt}) =>
          submitSignedEventOnce(
            wsUrl: wsUrl,
            nsec: nsec,
            kind: kind,
            content: content,
            tags: tags,
            createdAt: createdAt,
          ),
    );
    pushLeaseCleanupError.value = null;
  } catch (error, stackTrace) {
    // Community removal remains local-first. A failed best-effort tombstone is
    // observable here and the already-bounded relay lease expires naturally.
    reportPushLeaseCleanupError(error, stackTrace);
  }
}

class _CommunitySnapshotSync {
  _CommunitySnapshotSync(this._writer);

  final CommunitySnapshotWriter _writer;
  String? _lastSuccessfulSnapshot;

  Future<void> write(List<Community> communities) async {
    final fingerprint = communities
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
    if (fingerprint == _lastSuccessfulSnapshot) return;

    await _writer(communities);
    _lastSuccessfulSnapshot = fingerprint;
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
    await syncCommunitySnapshot(ref, communities);
    return communities;
  }

  /// Add a community. If one with the same relay URL already exists, update
  /// its credentials instead. Returns the effective community ID.
  Future<String> addCommunity(Community community) async {
    final storage = ref.read(communityStorageProvider);
    final current = state.value ?? [];

    // If a community with the same relay URL exists, update its credentials
    // instead of creating a duplicate entry.
    final existingIndex = current.indexWhere(
      (w) => w.relayUrl == community.relayUrl,
    );
    if (existingIndex >= 0) {
      final existing = current[existingIndex];
      final updated = existing.copyWith(
        pubkey: community.pubkey,
        nsec: community.nsec,
      );
      await storage.save(updated);
      final updatedList = [...current];
      updatedList[existingIndex] = updated;
      state = AsyncData(updatedList);
      await syncCommunitySnapshot(ref, updatedList);
      return existing.id;
    }

    await storage.save(community);
    final updatedList = [...current, community];
    state = AsyncData(updatedList);
    await syncCommunitySnapshot(ref, updatedList);
    return community.id;
  }

  Future<void> removeCommunity(String id) {
    return ref.read(communityTransitionProvider).runExclusive(() async {
      final storage = ref.read(communityStorageProvider);
      final activeId = await storage.loadActiveId();
      if (activeId == id) {
        await ref.read(communityTransitionProvider).run();
      }
      final current = state.value ?? await storage.loadAll();
      final removedIndex = current.indexWhere(
        (community) => community.id == id,
      );
      if (removedIndex >= 0) {
        await ref.read(communityPushLeaseDeactivatorProvider)(
          current[removedIndex],
        );
      }
      await storage.remove(id);

      final updatedList = current.where((w) => w.id != id).toList();
      state = AsyncData(updatedList);
      await syncCommunitySnapshot(ref, updatedList);

      // If we removed the active community, switch to another or sign out.
      if (activeId == id) {
        final remaining = state.value ?? [];
        if (remaining.isNotEmpty) {
          await storage.saveActiveId(remaining.first.id);
          // Reassign list state so activeCommunityProvider picks up the new ID.
          state = AsyncData([...remaining]);
          ref.invalidate(authProvider);
        } else {
          await storage.clearActiveId();
          // Invalidate auth so it re-evaluates against the now-empty storage
          // and transitions to unauthenticated.
          ref.invalidate(authProvider);
        }
      }
    });
  }

  Future<void> switchCommunity(String id) {
    return ref.read(communityTransitionProvider).runExclusive(() async {
      final storage = ref.read(communityStorageProvider);
      final activeId = await storage.loadActiveId();
      if (activeId == id) return;
      await ref.read(communityTransitionProvider).run();
      await storage.saveActiveId(id);
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

  Future<void> markPushLeaseAccepted(
    String id, {
    required List<BuzzPushSubscription> subscriptions,
    required int generation,
  }) => _serializePushMutation(() async {
    final storage = ref.read(communityStorageProvider);
    final current = state.value ?? await storage.loadAll();
    final index = current.indexWhere((community) => community.id == id);
    if (index < 0) return;

    final community = current[index];
    final acceptedGeneration =
        community.pushSubscriptionState.acceptedGeneration ?? 0;
    final generationCursor =
        community.pushSubscriptionState.generationCursor ?? 0;
    if (generation < max(acceptedGeneration, generationCursor)) return;
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
  });

  Future<void> setPushNotificationsEnabled(String id, bool enabled) async {
    Community? deactivation;
    int? tombstoneGeneration;
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
        tombstoneGeneration = cursor + 1;
        pushState = pushState.withReservedGeneration(tombstoneGeneration!);
      }
      final updated = community.copyWith(
        pushNotificationsEnabled: enabled,
        pushSubscriptionState: pushState,
      );
      await storage.save(updated);
      final updatedList = [...current]..[index] = updated;
      state = AsyncData(updatedList);
      await syncCommunitySnapshot(ref, updatedList);
      if (!enabled && tombstoneGeneration != null) deactivation = updated;
    });
    if (deactivation != null) {
      await ref.read(communityPushLeaseDeactivatorProvider)(
        deactivation!,
        generation: tombstoneGeneration,
      );
    }
  }

  Future<void> renameCommunity(String id, String name) async {
    final storage = ref.read(communityStorageProvider);
    final current = state.value ?? [];
    final index = current.indexWhere((w) => w.id == id);
    if (index < 0) return;

    final updated = current[index].copyWith(name: name);
    await storage.save(updated);

    final updatedList = [...current];
    updatedList[index] = updated;
    state = AsyncData(updatedList);
    await syncCommunitySnapshot(ref, updatedList);
  }
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

  if (activeId == null) {
    // No active ID stored but communities exist — fall back to first.
    await storage.saveActiveId(communities.first.id);
    return communities.first;
  }

  try {
    return communities.firstWhere((w) => w.id == activeId);
  } on StateError {
    // Active ID points to a community that no longer exists.
    // Fall back to first community.
    if (communities.isNotEmpty) {
      await storage.saveActiveId(communities.first.id);
      return communities.first;
    }
    return null;
  }
});
