import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/read_state/read_state_manager.dart';
import '../../../shared/read_state/read_state_provider.dart';
import '../../../shared/read_state/thread_follows_format.dart';
import '../../../shared/relay/relay.dart';
import '../../../shared/theme/theme_provider.dart';
import 'thread_follows_storage.dart';

class ThreadFollowsState {
  /// Explicitly followed thread roots.
  final Set<String> followedRootIds;

  /// Explicitly unfollowed thread roots: their ordinary replies never count,
  /// even in a thread the user started or replied in.
  final Set<String> unfollowedRootIds;

  /// The channel of each thread root with a choice, where known.
  final Map<String, String> channelByRoot;

  /// Whether some choices may be missing (see NIP-RS Buzz Thread Follow
  /// Choices).
  final bool incomplete;

  const ThreadFollowsState({
    this.followedRootIds = const {},
    this.unfollowedRootIds = const {},
    this.channelByRoot = const {},
    this.incomplete = true,
  });

  bool isFollowing(String rootId) => followedRootIds.contains(rootId);
}

/// Thread follow choices for the signed-in identity, synced across its
/// installations through read state. Each thread is automatic (it counts
/// when the user started or replied in it), followed, or unfollowed.
class ThreadFollowsNotifier extends Notifier<ThreadFollowsState> {
  // Pre-sync local follows, by root, still waiting for import.
  List<ThreadFollowEntry> _legacy = const [];
  // Channels known from verified events, by thread root.
  final Map<String, String> _channelByRoot = {};
  String? _pubkey;
  String? _relayUrl;

  @override
  ThreadFollowsState build() {
    final pubkey = ref.watch(myPubkeyProvider)?.trim().toLowerCase();
    final relayUrl = ref.watch(relayConfigProvider).baseUrl;
    final readState = ref.watch(readStateProvider);
    if (pubkey == null || pubkey.isEmpty) {
      _pubkey = null;
      _legacy = const [];
    } else if (_pubkey != pubkey || _relayUrl != relayUrl) {
      // Read state changes often; reload the pre-sync store only when the
      // identity or community does.
      _pubkey = pubkey;
      _relayUrl = relayUrl;
      _channelByRoot.clear();
      try {
        _legacy = ThreadFollowsStorage(
          ref.read(savedPrefsProvider),
        ).read(pubkey, relayUrl);
      } catch (_) {
        _legacy = const [];
      }
    }
    if (_legacy.isNotEmpty && !readState.threadFollowsIncomplete) {
      Future.microtask(_importLegacy);
    }
    final synced = {
      for (final key in readState.threadFollows.keys) threadFollowRoot(key),
    };
    return ThreadFollowsState(
      followedRootIds: {
        for (final MapEntry(:key, :value) in readState.threadFollows.entries)
          if (value.follow) threadFollowRoot(key),
        // A pre-sync follow not yet imported still shows; a synced choice
        // for the same root is newer and wins.
        for (final entry in _legacy)
          if (!synced.contains(entry.rootId)) entry.rootId,
      },
      unfollowedRootIds: {
        for (final MapEntry(:key, :value) in readState.threadFollows.entries)
          if (!value.follow) threadFollowRoot(key),
      },
      channelByRoot: {
        for (final entry in _legacy)
          entry.rootId: ?_channelByRoot[entry.rootId],
        for (final key in readState.threadFollows.keys)
          threadFollowRoot(key): threadFollowChannel(key),
      },
      incomplete: readState.threadFollowsIncomplete,
    );
  }

  Future<bool> followThread(String channelId, String rootId) =>
      _set(channelId, rootId, true);

  Future<bool> unfollowThread(String channelId, String rootId) =>
      _set(channelId, rootId, false);

  /// The failure to show when a follow or unfollow was refused.
  String get failureMessage {
    if (!ref.read(readStateProvider.notifier).threadFollowsReady) {
      return threadFollowsNotReadyMessage;
    }
    final error = ref.read(readStateProvider).syncError;
    return error is StateError && error.message == threadFollowsFullMessage
        ? threadFollowsFullMessage
        : 'Couldn’t save that. Try again.';
  }

  Future<bool> _set(String channelId, String rootId, bool follow) async {
    if (rootId.isEmpty) return false;
    return ref
        .read(readStateProvider.notifier)
        .setThreadFollow(channelId, rootId, follow);
  }

  /// Records that [rootId]'s thread is in [channelId], from a verified relay
  /// event, so a pre-sync follow saved without its channel can be imported.
  void noteThreadChannel(String rootId, String channelId) {
    if (!_legacy.any((entry) => entry.rootId == rootId)) return;
    if (_channelByRoot[rootId] == channelId) return;
    _channelByRoot[rootId] = channelId;
    // Callers may be mid-update of another provider.
    Future.microtask(_importLegacy);
  }

  /// Imports resolved pre-sync follows once, at `at = 0`; unresolved ones
  /// stay stored and are retried as their channels become known.
  Future<void> _importLegacy() async {
    final pubkey = _pubkey;
    if (!ref.mounted || pubkey == null || _legacy.isEmpty) return;
    if (ref.read(readStateProvider).threadFollowsIncomplete) return;
    final choices = <String, bool>{
      for (final entry in _legacy)
        if (_channelByRoot[entry.rootId] case final channelId?)
          threadFollowKey(channelId, entry.rootId): true,
    };
    if (choices.isEmpty) return;
    final settled =
        (await ref
                .read(readStateProvider.notifier)
                .importLegacyFollows(choices))
            .map(threadFollowRoot)
            .toSet();
    if (settled.isEmpty || !ref.mounted) return;
    _legacy = [
      for (final entry in _legacy)
        if (!settled.contains(entry.rootId)) entry,
    ];
    try {
      ThreadFollowsStorage(
        ref.read(savedPrefsProvider),
      ).write(pubkey, _relayUrl!, _legacy);
    } catch (_) {
      // Retried from the remaining entries next time.
    }
  }
}

final threadFollowsProvider =
    NotifierProvider<ThreadFollowsNotifier, ThreadFollowsState>(
      ThreadFollowsNotifier.new,
    );
