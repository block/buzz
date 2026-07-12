import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../workspace/workspace.dart';
import '../workspace/workspace_provider.dart';

enum AuthStatus { unknown, unauthenticated, authenticated, offline }

class AuthState {
  final AuthStatus status;
  final Workspace? workspace;

  const AuthState({required this.status, this.workspace});
}

/// Restores the active workspace without making connectivity load-bearing.
/// The relay session owns connection recovery after startup.
class AuthNotifier extends AsyncNotifier<AuthState> {
  @override
  Future<AuthState> build() async {
    // Read from storage directly — NOT from workspace providers.
    // Watching workspace providers here would create a circular dependency
    // because authenticateWithWorkspace() writes to those providers.
    final storage = ref.read(workspaceStorageProvider);
    final workspaces = await storage.loadAll();
    if (workspaces.isEmpty) {
      return const AuthState(status: AuthStatus.unauthenticated);
    }

    var activeId = await storage.loadActiveId();
    while (workspaces.isNotEmpty) {
      final active = activeId != null && workspaces.any((w) => w.id == activeId)
          ? workspaces.firstWhere((w) => w.id == activeId)
          : workspaces.first;
      await storage.saveActiveId(active.id);

      if (_hasValidNsec(active.nsec)) {
        return AuthState(status: AuthStatus.authenticated, workspace: active);
      }

      await storage.remove(active.id);
      workspaces.removeWhere((workspace) => workspace.id == active.id);
      activeId = null;
      ref.invalidate(workspaceListProvider);
      ref.invalidate(activeWorkspaceProvider);
    }

    await storage.clearActiveId();
    return const AuthState(status: AuthStatus.unauthenticated);
  }

  /// Reload the active workspace after a startup error.
  Future<void> retry() async {
    ref.invalidateSelf();
    await future;
  }

  /// Authenticate with a workspace. Saves it and switches to it.
  /// Writes to storage directly to avoid circular dependency with workspace
  /// providers.
  Future<void> authenticateWithWorkspace(Workspace workspace) async {
    final storage = ref.read(workspaceStorageProvider);
    await storage.save(workspace);
    await storage.saveActiveId(workspace.id);

    // Invalidate workspace providers so other consumers pick up the new data.
    ref.invalidate(workspaceListProvider);
    ref.invalidate(activeWorkspaceProvider);

    state = AsyncData(
      AuthState(status: AuthStatus.authenticated, workspace: workspace),
    );
  }

  Future<void> signOut() async {
    final storage = ref.read(workspaceStorageProvider);
    final activeId = await storage.loadActiveId();
    if (activeId != null) {
      await storage.remove(activeId);
      await storage.clearActiveId();
    }

    // Check if other workspaces remain — switch to the next one instead of
    // forcing the user back to the pairing screen.
    final remaining = await storage.loadAll();

    // Invalidate workspace providers so other consumers pick up the change.
    ref.invalidate(workspaceListProvider);
    ref.invalidate(activeWorkspaceProvider);

    if (remaining.isNotEmpty) {
      final next = remaining.first;
      await storage.saveActiveId(next.id);
      // Re-run build() to validate the next workspace's credentials.
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
