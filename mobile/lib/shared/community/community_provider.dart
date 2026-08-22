import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../auth/auth_provider.dart';
import '../push/dev_push_lease.dart';
import '../push/push_bridge.dart';
import '../push/push_capability.dart';
import '../push/push_subscription.dart';
import '../relay/signed_event_relay.dart';
import 'community.dart';
import 'community_storage.dart';

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
    Future<void> Function(Community community);

final communityPushLeaseDeactivatorProvider =
    Provider<CommunityPushLeaseDeactivator>((ref) {
      return (community) => _deactivateCommunityPushLease(community);
    });

Future<void> _deactivateCommunityPushLease(Community community) async {
  if (!buzzPushCapabilityEnabled) return;
  final state = community.pushSubscriptionState;
  final acceptedGeneration = state.acceptedGeneration;
  final installationId = state.acceptedInstallationId;
  final nsec = community.nsec;
  if (acceptedGeneration == null ||
      installationId == null ||
      nsec == null ||
      nsec.isEmpty) {
    return;
  }
  try {
    final decoded = nostr.Nip19.decode(payload: nsec);
    final memberPubkey = community.pubkey ?? nostr.Keys(decoded.data).public;
    final descriptor = await fetchBuzzPushLeaseDescriptor(community.relayUrl);
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
    final generation = acceptedGeneration + 2;
    await publishBuzzPushLeaseTombstone(
      descriptor: descriptor,
      installationId: installationId,
      generation: generation,
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

  Future<void> removeCommunity(String id) async {
    final storage = ref.read(communityStorageProvider);
    final current = state.value ?? await storage.loadAll();
    final removedIndex = current.indexWhere((community) => community.id == id);
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
    final activeId = await storage.loadActiveId();
    if (activeId == id) {
      final remaining = state.value ?? [];
      if (remaining.isNotEmpty) {
        await switchCommunity(remaining.first.id);
      } else {
        await storage.clearActiveId();
        // Invalidate auth so it re-evaluates against the now-empty storage
        // and transitions to unauthenticated.
        ref.invalidate(authProvider);
      }
    }
  }

  Future<void> switchCommunity(String id) async {
    final storage = ref.read(communityStorageProvider);
    await storage.saveActiveId(id);
    // Reassign list state to trigger activeCommunityProvider (which watches
    // communityListProvider.future) to rebuild and pick up the new active ID.
    // We can't use ref.invalidate(activeCommunityProvider) here because that
    // creates a circular dependency — activeCommunityProvider watches us.
    state = AsyncData([...state.value ?? []]);
    // Invalidate auth so AuthState.community reflects the new active community.
    ref.invalidate(authProvider);
  }

  Future<void> updateDesiredPushSubscriptions(
    String id,
    List<BuzzPushSubscription> desired,
  ) async {
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
  }

  Future<void> markPushLeaseAccepted(
    String id, {
    required List<BuzzPushSubscription> subscriptions,
    required int generation,
    required int grantGeneration,
    required String installationId,
  }) async {
    final storage = ref.read(communityStorageProvider);
    final current = state.value ?? await storage.loadAll();
    final index = current.indexWhere((community) => community.id == id);
    if (index < 0) return;

    final community = current[index];
    final updated = community.copyWith(
      pushSubscriptionState: community.pushSubscriptionState.withAccepted(
        subscriptions: subscriptions,
        generation: generation,
        grantGeneration: grantGeneration,
        installationId: installationId,
      ),
    );
    await storage.save(updated);
    final updatedList = [...current]..[index] = updated;
    state = AsyncData(updatedList);
    await syncCommunitySnapshot(ref, updatedList);
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
