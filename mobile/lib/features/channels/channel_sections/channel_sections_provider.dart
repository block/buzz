import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/relay/relay.dart';
import '../../../shared/theme/theme_provider.dart';
import '../../../shared/community/community_provider.dart';
import '../../../shared/me/me_sync.dart';
import 'channel_sections_manager.dart';
import 'channel_sections_storage.dart';
import 'workspace_layout.dart';

class ChannelSectionsState {
  final bool isReady;
  final ChannelSectionStore store;

  /// Desktop's unified layout head, when it has saved one. Legacy group edits
  /// no longer reach desktop once it exists.
  final WorkspaceRecord? workspace;

  /// Bumped on every change to force downstream rebuilds.
  final int version;

  const ChannelSectionsState({
    this.isReady = false,
    this.store = const ChannelSectionStore(),
    this.workspace,
    this.version = 0,
  });

  bool get editable => workspace == null;
}

class ChannelSectionsNotifier extends Notifier<ChannelSectionsState> {
  ChannelSectionsManager? _manager;
  bool _isInitialized = false;

  @override
  ChannelSectionsState build() {
    _manager?.dispose(flushPending: false);
    _manager = null;
    _isInitialized = false;

    final relayConfig = ref.watch(relayConfigProvider);
    final sessionState = ref.watch(relaySessionProvider);
    // Rebuild when the active community changes (pubkey may differ).
    ref.watch(activeCommunityProvider);

    final nsec = relayConfig.nsec?.trim();
    if (nsec == null || nsec.isEmpty) {
      return const ChannelSectionsState();
    }

    final pubkey = ref.watch(myPubkeyProvider);
    final crypto = ref.watch(channelSectionsCryptoProvider);
    if (pubkey == null || crypto == null) {
      return const ChannelSectionsState();
    }

    final prefs = ref.read(savedPrefsProvider);
    final signedRelay = SignedEventRelay(
      session: ref.read(relaySessionProvider.notifier),
      nsec: nsec,
    );

    late final ChannelSectionsManager manager;
    manager = ChannelSectionsManager(
      pubkey: pubkey,
      relayUrl: relayConfig.baseUrl,
      prefs: prefs,
      crypto: crypto,
      relaySession: ref.read(relaySessionProvider.notifier),
      signedEventRelay: signedRelay,
      remoteEnabled: sessionState.status == SessionStatus.connected,
      onChanged: () => _emitManagerState(manager),
    );
    _manager = manager;

    // Resume re-read catches an EVENT a healthy socket never delivered.
    ref.listen(appForegroundProvider, (_, foreground) {
      if (foreground) manager.refreshFromRelay();
    });

    ref.onDispose(() {
      manager.dispose();
      if (_manager == manager) {
        _manager = null;
      }
    });

    Future.microtask(() async {
      await manager.initialize();
      if (_manager != manager) return;
      _isInitialized = true;
      _emitManagerState(manager);
    });

    return ChannelSectionsState(
      isReady: false,
      store: manager.store,
      workspace: manager.workspace,
      version: 1,
    );
  }

  /// Files [channelId] in the new section in the same write, when given.
  void createSection(String name, {String? channelId}) =>
      _edit((m) => m.createSection(name, channelId: channelId));

  void renameSection(String sectionId, String? newName, {String? icon}) =>
      _manager?.renameSection(sectionId, newName, icon: icon);

  void deleteSection(String sectionId) {
    final manager = _manager;
    if (manager == null) return;
    manager.deleteSection(sectionId);
    _emitManagerState(manager);
  }

  void moveSectionUp(String sectionId) => _manager?.moveSectionUp(sectionId);

  void moveSectionDown(String sectionId) =>
      _manager?.moveSectionDown(sectionId);

  void assignChannel(String channelId, String sectionId) =>
      _edit((m) => m.assignChannel(channelId, sectionId));

  void unassignChannel(String channelId) =>
      _edit((m) => m.unassignChannel(channelId));

  /// Applies a local edit and emits it at once, as [deleteSection] does,
  /// rather than waiting for the relay to echo it back.
  void _edit(void Function(ChannelSectionsManager manager) edit) {
    final manager = _manager;
    if (manager == null) return;
    edit(manager);
    _emitManagerState(manager);
  }

  void _emitManagerState(ChannelSectionsManager manager) {
    if (_manager != manager) return;
    state = ChannelSectionsState(
      isReady: _isInitialized,
      store: manager.store,
      workspace: manager.workspace,
      version: state.version + 1,
    );
  }
}

final channelSectionsProvider =
    NotifierProvider<ChannelSectionsNotifier, ChannelSectionsState>(
      ChannelSectionsNotifier.new,
    );

/// Identity-scoped crypto survives connection status changes. A community or
/// signing-key change replaces it through [relayConfigProvider].
final channelSectionsCryptoProvider = Provider<ChannelSectionsCrypto?>((ref) {
  final config = ref.watch(relayConfigProvider);
  final pubkey = ref.watch(myPubkeyProvider);
  final nsec = config.nsec?.trim();
  if (nsec == null || nsec.isEmpty || pubkey == null) return null;
  try {
    return ChannelSectionsCrypto(nsec, pubkey);
  } catch (_) {
    return null;
  }
});

/// The workspaces and section order the desktop sidebar renders for this
/// account and community.
final sidebarLayoutProvider = Provider<WorkspaceLayout>((ref) {
  final sections = ref.watch(channelSectionsProvider);
  return effectiveWorkspaceLayout(
    unified: sections.workspace,
    team: sections.store,
    solo: ref.watch(meGroupsProvider).value,
  );
});
