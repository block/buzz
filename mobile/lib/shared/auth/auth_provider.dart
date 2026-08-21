import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../community/community.dart';
import '../community/community_provider.dart';

enum AuthStatus { unknown, unauthenticated, authenticated }

class AuthState {
  final AuthStatus status;
  final Community? community;

  const AuthState({required this.status, this.community});
}

/// Restores the active community without making connectivity load-bearing.
/// The relay session owns connection recovery after startup.
class AuthNotifier extends AsyncNotifier<AuthState> {
  @override
  Future<AuthState> build() async {
    // Read from storage directly — NOT from community providers.
    // Watching community providers here would create a circular dependency
    // because authenticateWithCommunity() writes to those providers.
    final storage = ref.read(communityStorageProvider);
    final communities = await storage.loadAll();
    if (communities.isEmpty) {
      return const AuthState(status: AuthStatus.unauthenticated);
    }

    var activeId = await storage.loadActiveId();
    while (communities.isNotEmpty) {
      final active =
          activeId != null && communities.any((w) => w.id == activeId)
          ? communities.firstWhere((w) => w.id == activeId)
          : communities.first;
      await storage.saveActiveId(active.id);

      if (_hasValidNsec(active.nsec)) {
        return AuthState(status: AuthStatus.authenticated, community: active);
      }

      await storage.remove(active.id);
      communities.removeWhere((community) => community.id == active.id);
      activeId = null;
      ref.invalidate(communityListProvider);
      ref.invalidate(activeCommunityProvider);
    }

    await storage.clearActiveId();
    return const AuthState(status: AuthStatus.unauthenticated);
  }

  /// Authenticate with a community. Saves it and switches to it.
  /// Writes to storage directly to avoid circular dependency with community
  /// providers.
  Future<void> authenticateWithCommunity(Community community) async {
    final storage = ref.read(communityStorageProvider);
    await storage.save(community);
    await storage.saveActiveId(community.id);

    // Invalidate community providers so other consumers pick up the new data.
    ref.invalidate(communityListProvider);
    ref.invalidate(activeCommunityProvider);

    state = AsyncData(
      AuthState(status: AuthStatus.authenticated, community: community),
    );
  }

  Future<void> signOut() async {
    final storage = ref.read(communityStorageProvider);
    final activeId = await storage.loadActiveId();
    if (activeId != null) {
      await storage.remove(activeId);
      await storage.clearActiveId();
    }

    // Check if other communities remain — switch to the next one instead of
    // forcing the user back to the pairing screen.
    final remaining = await storage.loadAll();

    // Invalidate community providers so other consumers pick up the change.
    ref.invalidate(communityListProvider);
    ref.invalidate(activeCommunityProvider);

    if (remaining.isNotEmpty) {
      final next = remaining.first;
      await storage.saveActiveId(next.id);
      // Re-run build() to validate the next community's credentials.
      ref.invalidateSelf();
      await future;
    } else {
      state = const AsyncData(AuthState(status: AuthStatus.unauthenticated));
    }
  }
}

bool _hasValidNsec(String? nsec) {
  if (nsec == null || nsec.isEmpty) return false;
  try {
    final decoded = nostr.Nip19.decode(payload: nsec);
    return decoded.prefix == nostr.Nip19Prefix.nsec &&
        decoded.data.length == 64;
  } catch (_) {
    return false;
  }
}

final authProvider = AsyncNotifierProvider<AuthNotifier, AuthState>(
  AuthNotifier.new,
);
