import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:isolate';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:path/path.dart' as p;

import '../community/community_provider.dart';
import '../relay/media_disk_cache.dart';
import '../relay/nostr_models.dart';
import '../relay/relay_provider.dart' show pubkeyFromNsec;

const _version = 1;

/// Saved directories beyond these bounds are discarded on read and never
/// written; the directory then loads from the relay.
const maxSavedDirectoryChannels = 20000;
const maxSavedDirectoryBytes = 16 * 1024 * 1024;

/// One community's saved open-channel directory.
class SavedOpenChannelDirectory {
  const SavedOpenChannelDirectory({
    required this.metas,
    required this.fullAt,
    required this.readAt,
  });

  /// The latest kind:39000 metadata per channel, without signatures: the
  /// relay signed them, and the copy is only a local cache of what it served.
  final List<NostrEvent> metas;

  /// When the relay last served the whole directory, in Unix seconds.
  final int fullAt;

  /// When the last read that changed [metas] started, by the device clock, in
  /// Unix seconds. Catch-ups never start after it: the relay may stamp
  /// [metas] ahead of when it served them.
  final int readAt;
}

/// Saves one community and account's open-channel directory in a file.
///
/// The data is a cache derived from relay events: unreadable, unsupported or
/// oversized data is discarded rather than migrated. [close] retires the
/// store, so a write in flight when the community or account goes away does
/// not recreate the file.
class OpenChannelDirectoryStorage {
  OpenChannelDirectoryStorage(Future<File?> file)
    // Readers wait for the file, so never wait on it for long; an
    // unavailable caches directory reads as no saved copy.
    : _file = file.timeout(_openTimeout, onTimeout: () => null).catchError((
        Object error,
      ) {
        debugPrint('[OpenChannelDirectoryStorage] no file: $error');
        return null;
      });

  static const _openTimeout = Duration(seconds: 3);

  final Future<File?> _file;
  bool _closed = false;
  Future<void> _writes = Future.value();

  /// Where [communityId]'s directory for the account [signer] is saved.
  static File fileFor(Directory root, String communityId, String signer) =>
      File(
        p.join(
          MediaDiskCache.communityDirectory(root, communityId).path,
          'channel-directory',
          '${signer.toLowerCase()}.json',
        ),
      );

  /// Deletes every saved directory under [root] except each retained
  /// community's file for its current signer (community id to signer), and
  /// the pending copy a save of it may be writing. A
  /// failed delete is retried on the next call. Removed communities lose their
  /// whole directory to [MediaDiskCache.removeOrphans].
  static Future<void> removeOrphans(
    Directory root,
    Map<String, String?> Function() retained,
  ) async {
    final communities = Directory(p.join(root.path, 'communities'));
    if (!await communities.exists()) return;
    Set<String> keep() => {
      for (final MapEntry(key: id, value: signer) in retained().entries)
        if (signer != null) ...{
          fileFor(root, id, signer).path,
          '${fileFor(root, id, signer).path}.pending',
        },
    };
    await for (final community in communities.list(followLinks: false)) {
      if (community is! Directory) continue;
      final saved = Directory(p.join(community.path, 'channel-directory'));
      if (!await saved.exists()) continue;
      await for (final entry in saved.list(followLinks: false)) {
        if (keep().contains(entry.path)) continue;
        try {
          await entry.delete(recursive: true);
        } on FileSystemException catch (error) {
          debugPrint('[OpenChannelDirectoryStorage] cleanup failed: $error');
        }
      }
    }
  }

  /// The saved directory, or null when there is none or it is unusable.
  Future<SavedOpenChannelDirectory?> read() async {
    final file = await _file;
    if (file == null || _closed) return null;
    final Uint8List raw;
    try {
      if (!await file.exists()) return null;
      if (await file.length() > maxSavedDirectoryBytes) {
        throw const FormatException('size');
      }
      raw = await file.readAsBytes();
    } on FileSystemException catch (error) {
      debugPrint('[OpenChannelDirectoryStorage] read failed: $error');
      return null;
    } on FormatException {
      await _discard(file);
      return null;
    }
    // Up to 16 MiB of JSON: decode off the UI isolate.
    final saved = await Isolate.run(() => _decode(raw));
    if (saved == null) await _discard(file);
    return _closed ? null : saved;
  }

  /// Replaces the saved directory. Writes run one at a time, in call order.
  /// Returns false when the store was closed or the directory exceeds the
  /// bounds, in which case no stale copy is left behind.
  Future<bool> write(SavedOpenChannelDirectory directory) {
    final result = _writes.then((_) => _write(directory));
    _writes = result.then((_) {}, onError: (_) {});
    return result;
  }

  Future<bool> _write(SavedOpenChannelDirectory directory) async {
    final file = await _file;
    if (file == null || _closed) return false;
    // Up to 20,000 channels: encode off the UI isolate.
    final raw = await Isolate.run(() => _encode(directory));
    if (_closed) return false;
    if (raw == null) {
      // An older, smaller copy no longer describes the directory.
      await _discard(file);
      return false;
    }
    await file.parent.create(recursive: true);
    // Write beside the file and rename over it, so a crash mid-write leaves
    // the previous copy rather than a torn one.
    final pending = File('${file.path}.pending');
    await pending.writeAsBytes(raw, flush: true);
    if (_closed) {
      await pending.delete();
      return false;
    }
    await pending.rename(file.path);
    return true;
  }

  /// Stops every later read and write, as when the community or account is
  /// retired. The file itself is deleted by [removeOrphans].
  void close() => _closed = true;

  Future<void> _discard(File file) async {
    try {
      if (await file.exists()) await file.delete();
    } on FileSystemException catch (error) {
      debugPrint('[OpenChannelDirectoryStorage] discard failed: $error');
    }
  }
}

/// The file's UTF-8 JSON, or null when [directory] exceeds the bounds.
Uint8List? _encode(SavedOpenChannelDirectory directory) {
  final metas = directory.metas;
  if (metas.length > maxSavedDirectoryChannels) return null;
  final bytes = utf8.encode(
    jsonEncode({
      'v': _version,
      'full_at': directory.fullAt,
      'read_at': directory.readAt,
      'metas': [
        for (final event in metas)
          {
            'id': event.id,
            'pubkey': event.pubkey,
            'created_at': event.createdAt,
            'tags': event.tags,
            'content': event.content,
          },
      ],
    }),
  );
  return bytes.length > maxSavedDirectoryBytes ? null : bytes;
}

SavedOpenChannelDirectory? _decode(Uint8List raw) {
  try {
    final json = jsonDecode(utf8.decode(raw)) as Map<String, dynamic>;
    if (json['v'] != _version) return null;
    final metas = json['metas'] as List<dynamic>;
    if (metas.length > maxSavedDirectoryChannels) return null;
    return SavedOpenChannelDirectory(
      fullAt: json['full_at'] as int,
      readAt: json['read_at'] as int,
      metas: [
        for (final meta in metas.cast<Map<String, dynamic>>())
          NostrEvent(
            id: meta['id'] as String,
            pubkey: meta['pubkey'] as String,
            createdAt: meta['created_at'] as int,
            kind: 39000,
            tags: [
              for (final tag in meta['tags'] as List<dynamic>)
                (tag as List<dynamic>).cast<String>().toList(),
            ],
            content: meta['content'] as String,
            sig: '',
          ),
      ],
    );
  } on Object {
    return null;
  }
}

/// The active community's saved directory, or null when there is no caches
/// root (isolated tests, or no app group) or no community. Switching
/// community or account closes the previous one.
final openChannelDirectoryStorageProvider =
    Provider<OpenChannelDirectoryStorage?>((ref) {
      final (communityId, signer) = ref.watch(
        activeCommunityProvider.select((value) {
          final community = value.value;
          if (community == null) return (null, null);
          final signer =
              pubkeyFromNsec(community.nsec?.trim()) ?? community.pubkey;
          return (community.id, signer);
        }),
      );
      if (communityId == null || signer == null) return null;
      final storage = OpenChannelDirectoryStorage(
        ref
            .watch(mediaCacheRootProvider.future)
            .then(
              (root) => root == null
                  ? null
                  : OpenChannelDirectoryStorage.fileFor(
                      root,
                      communityId,
                      signer,
                    ),
            ),
      );
      ref.onDispose(storage.close);
      return storage;
    });
