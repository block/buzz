import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

/// A file to pre-attach in a channel composer the next time it mounts.
@immutable
class ComposerSeedFile {
  final String path;
  final String? name;
  final String? mimeType;

  const ComposerSeedFile({required this.path, this.name, this.mimeType});

  bool get isImage => mimeType?.startsWith('image/') ?? false;
  bool get isVideo => mimeType?.startsWith('video/') ?? false;

  @override
  bool operator ==(Object other) =>
      other is ComposerSeedFile &&
      other.path == path &&
      other.name == name &&
      other.mimeType == mimeType;

  @override
  int get hashCode => Object.hash(path, name, mimeType);
}

/// Attachments parked for a channel composer, keyed by channel id.
///
/// Text drafts already flow through the persisted compose-drafts store; this
/// only carries files (share-sheet hand-offs) that must not be persisted. A
/// seed is taken exactly once, by the first composer for that channel to mount
/// after it was planted. Ownership of the files transfers with it: the
/// composer deletes them after upload or discard.
class ComposerSeedNotifier
    extends Notifier<Map<String, List<ComposerSeedFile>>> {
  @override
  Map<String, List<ComposerSeedFile>> build() => const {};

  void plant(String channelId, List<ComposerSeedFile> files) {
    if (files.isEmpty) return;
    state = {...state, channelId: List.unmodifiable(files)};
  }

  List<ComposerSeedFile> take(String channelId) {
    final files = state[channelId];
    if (files == null) return const [];
    state = {
      for (final entry in state.entries)
        if (entry.key != channelId) entry.key: entry.value,
    };
    return files;
  }
}

final composerSeedProvider =
    NotifierProvider<ComposerSeedNotifier, Map<String, List<ComposerSeedFile>>>(
      ComposerSeedNotifier.new,
    );
