import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/relay/relay.dart';
import '../../../shared/theme/theme_provider.dart';
import '../read_state/read_state_time.dart';
import 'thread_follows_storage.dart';

class ThreadFollowsState {
  final Set<String> followedRootIds;

  const ThreadFollowsState({this.followedRootIds = const {}});

  bool isFollowing(String rootId) => followedRootIds.contains(rootId);
}

/// Followed thread roots for the signed-in identity. Follows feed the unread
/// pipeline (`shouldNotifyForEvent`'s `followedRootIds`), so replies in a
/// followed thread badge the channel even without participation.
class ThreadFollowsNotifier extends Notifier<ThreadFollowsState> {
  List<ThreadFollowEntry> _entries = const [];
  String? _pubkey;

  @override
  ThreadFollowsState build() {
    final pubkey = ref.watch(myPubkeyProvider)?.trim().toLowerCase();
    _pubkey = pubkey;
    if (pubkey == null || pubkey.isEmpty) {
      _entries = const [];
      return const ThreadFollowsState();
    }

    try {
      _entries = ThreadFollowsStorage(
        ref.read(savedPrefsProvider),
      ).read(pubkey);
    } catch (_) {
      _entries = const [];
    }
    return _stateFromEntries();
  }

  void followThread(String rootId) {
    if (rootId.isEmpty || _entries.any((e) => e.rootId == rootId)) return;
    _entries = capThreadFollowEntries([
      ..._entries,
      ThreadFollowEntry(rootId: rootId, followedAt: currentUnixSeconds()),
    ]);
    _persist();
    state = _stateFromEntries();
  }

  void unfollowThread(String rootId) {
    final next = [
      for (final entry in _entries)
        if (entry.rootId != rootId) entry,
    ];
    if (next.length == _entries.length) return;
    _entries = next;
    _persist();
    state = _stateFromEntries();
  }

  void _persist() {
    final pubkey = _pubkey;
    if (pubkey == null || pubkey.isEmpty) return;
    try {
      ThreadFollowsStorage(
        ref.read(savedPrefsProvider),
      ).write(pubkey, _entries);
    } catch (_) {
      // In-memory state still works this session.
    }
  }

  ThreadFollowsState _stateFromEntries() => ThreadFollowsState(
    followedRootIds: Set.unmodifiable({for (final e in _entries) e.rootId}),
  );
}

final threadFollowsProvider =
    NotifierProvider<ThreadFollowsNotifier, ThreadFollowsState>(
      ThreadFollowsNotifier.new,
    );
