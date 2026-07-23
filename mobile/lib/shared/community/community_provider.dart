import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../auth/auth_provider.dart';
import 'community.dart';
import 'community_storage.dart';

final communityStorageProvider = Provider<CommunityStorage>((ref) {
  return CommunityStorage();
});

class CommunityListNotifier extends AsyncNotifier<List<Community>> {
  @override
  Future<List<Community>> build() async {
    final storage = ref.read(communityStorageProvider);
    return storage.loadAll();
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
      return existing.id;
    }

    await storage.save(community);
    state = AsyncData([...current, community]);
    return community.id;
  }

  Future<void> removeCommunity(String id) async {
    final storage = ref.read(communityStorageProvider);
    await storage.remove(id);

    final current = state.value ?? [];
    state = AsyncData(current.where((w) => w.id != id).toList());

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
