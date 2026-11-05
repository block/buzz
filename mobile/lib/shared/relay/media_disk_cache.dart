import 'dart:async';
import 'dart:io';

import 'package:file/file.dart' as pf;
import 'package:file/local.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_cache_manager/flutter_cache_manager.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:path/path.dart' as p;
import 'package:path_provider/path_provider.dart';

import '../community/community_provider.dart';
import 'relay_provider.dart' show pubkeyFromNsec;

/// Image bytes saved on disk, and whether they are due a background refresh.
typedef MediaDiskCacheHit = ({Uint8List bytes, bool isStale});

/// Persistent store for one community's remote image bytes: custom emoji,
/// avatars, attachments and anything else [MediaImageProvider] loads.
///
/// Built on `flutter_cache_manager`. It keeps the bytes each URL served, so a
/// relay thumbnail and its original are separate entries. Each community and
/// account has its own directory under the caches root (see
/// [mediaCacheRootProvider]), deleted with the community or account. Least recently used files are evicted once
/// the directory holds more than [maxBytes].
///
/// Relay media URLs are content hashes and never go stale. Other URLs can
/// change in place, so they are served from disk but refetched in the
/// background once older than [externalRefreshAge].
class MediaDiskCache {
  MediaDiskCache._(this._manager, this._now);

  /// Opens the store in [directory], creating it on first write. [now] is
  /// the clock staleness is judged by.
  factory MediaDiskCache.open(
    Directory directory, {
    @visibleForTesting DateTime Function() now = DateTime.now,
  }) {
    final index = File(p.join(directory.path, 'index.json'));
    return MediaDiskCache._(
      CacheManager(
        Config(
          directory.path,
          // Age never evicts: the byte budget below does.
          stalePeriod: _neverStale,
          maxNrOfCacheObjects: maxFiles,
          repo: MediaCacheIndex(index, directory, maxBytes),
          fileSystem: _DirectoryFileSystem(directory),
        ),
      ),
      now,
    );
  }

  /// Most bytes one community's cache keeps on disk.
  static const maxBytes = 250 * 1024 * 1024;

  /// Most files one community's cache keeps, so many tiny files (emoji,
  /// avatars) cannot cost far more disk and index work than their bytes.
  static const maxFiles = 10000;

  /// Larger images still show, from memory, but are never written to disk,
  /// so one image cannot evict the rest of the cache.
  static const maxFileBytes = 5 * 1024 * 1024;

  /// Age at which a non-relay image is refetched in the background.
  static const externalRefreshAge = Duration(days: 7);

  static const _neverStale = Duration(days: 365 * 100);

  final CacheManager _manager;
  final DateTime Function() _now;
  var _closed = false;

  /// The directory holding everything one community persists under [root].
  static Directory communityDirectory(Directory root, String communityId) =>
      Directory(
        p.join(root.path, 'communities', Uri.encodeComponent(communityId)),
      );

  /// Where [communityId]'s image cache for the account [signer] lives under
  /// [root]. Each account has its own, so one account's images never serve
  /// another's.
  static Directory directoryFor(
    Directory root,
    String communityId,
    String signer,
  ) => Directory(
    p.join(communityDirectory(root, communityId).path, 'media', signer),
  );

  /// Deletes every community directory under [root] whose id is not in
  /// [retainedIds]: a removed community's data, including writes that landed
  /// after its removal. A failed delete is retried on the next call.
  static Future<void> removeOrphans(
    Directory root,
    Set<String> retainedIds,
  ) async {
    final communities = Directory(p.join(root.path, 'communities'));
    if (!await communities.exists()) return;
    final retained = {
      for (final id in retainedIds) communityDirectory(root, id).path,
    };
    await for (final entry in communities.list(followLinks: false)) {
      if (entry is! Directory || retained.contains(entry.path)) continue;
      try {
        await entry.delete(recursive: true);
      } on FileSystemException catch (error) {
        debugPrint('[MediaDiskCache] orphan cleanup failed: $error');
      }
    }
  }

  /// [url]'s saved bytes, or null when they are not on disk.
  Future<MediaDiskCacheHit?> read(String url) async {
    final FileInfo? info;
    final Uint8List bytes;
    try {
      info = await _manager.getFileFromCache(url);
      if (info == null) return null;
      bytes = await info.file.readAsBytes();
    } on FileSystemException {
      // The OS may purge cache files under the index. Drop the entry and
      // load from the network.
      unawaited(_manager.removeFile(url).catchError(_logFailure));
      return null;
    }
    if (bytes.isEmpty) return null;
    return (bytes: bytes, isStale: info.validTill.isBefore(_now()));
  }

  /// Saves [bytes] for [url]. [immutable] bytes (relay media) are never
  /// refetched; others are refetched after [externalRefreshAge]. Bytes over
  /// [maxFileBytes] are not saved, and drop any older copy so it is not
  /// shown in their place. Failures are logged, never thrown: a missing
  /// entry only costs a later download.
  Future<void> write(
    String url,
    Uint8List bytes, {
    required bool immutable,
  }) async {
    // A download that finishes after its account or community was retired
    // must not repopulate the cache.
    if (_closed || bytes.isEmpty) return;
    try {
      if (bytes.length > maxFileBytes) {
        await _manager.removeFile(url);
        return;
      }
      await _manager.putFile(
        url,
        bytes,
        maxAge: immutable ? _neverStale : externalRefreshAge,
      );
    } on Object catch (error) {
      _logFailure(error);
    }
  }

  /// Stops writes, then flushes the index.
  Future<void> dispose() {
    _closed = true;
    return _manager.dispose();
  }

  static void _logFailure(Object error) =>
      debugPrint('[MediaDiskCache] write failed: $error');
}

/// `flutter_cache_manager` caps a cache only by file count. This index keeps
/// its JSON format but evicts by bytes: once the files total more than
/// [_maxBytes], the least recently used are over capacity.
@visibleForTesting
class MediaCacheIndex extends JsonCacheInfoRepository {
  MediaCacheIndex(super.file, this._directory, this._maxBytes)
    : super.withFile();

  final Directory _directory;
  final int _maxBytes;

  /// Least recently used files past [capacity] files or past the byte
  /// budget, whichever keeps fewer.
  @override
  Future<List<CacheObject>> getObjectsOverCapacity(int capacity) async {
    final newestFirst = await getAllObjects()
      ..sort((a, b) => b.touched!.compareTo(a.touched!));
    var total = 0;
    var count = 0;
    return [
      for (final object in newestFirst)
        if ((total += object.length ?? 0) > _maxBytes || ++count > capacity)
          object,
    ];
  }

  /// `putFile` records no length, and replacing a file's bytes keeps the old
  /// one, so measure the file on every write.
  @override
  Future<dynamic> updateOrInsert(CacheObject cacheObject) async {
    final file = File(p.join(_directory.path, cacheObject.relativePath));
    final length = await file.exists() ? await file.length() : 0;
    return super.updateOrInsert(cacheObject.copyWith(length: length));
  }
}

/// Keeps cache files in one directory of our choosing rather than the
/// package's default under the temporary directory.
class _DirectoryFileSystem implements FileSystem {
  _DirectoryFileSystem(Directory directory)
    : _directory = const LocalFileSystem().directory(directory.path);

  final pf.Directory _directory;

  @override
  Future<pf.File> createFile(String name) async {
    await _directory.create(recursive: true);
    return _directory.childFile(name);
  }
}

const _appGroupChannel = MethodChannel('buzz/app_group');

/// The directory community caches live under: the shared app group's caches
/// on iOS, so app extensions can read them, and the app's cache directory
/// elsewhere. Null when the iOS app group is unavailable.
Future<Directory?> resolveMediaCacheRoot() async {
  if (!Platform.isIOS) return getApplicationCacheDirectory();
  final path = await _appGroupChannel.invokeMethod<String>('cachesDirectory');
  if (path == null) {
    debugPrint('[MediaDiskCache] no app group container; disk cache is off');
    return null;
  }
  return Directory(path);
}

/// The caches root, overridden at startup with [resolveMediaCacheRoot]. Null
/// (isolated tests, or no app group) turns the disk cache off. Startup does
/// not wait for it: images that load first skip the disk.
final mediaCacheRootProvider = FutureProvider<Directory?>((ref) async => null);

/// The active community's image cache, or null when there is no caches root
/// or no community. Switching community or account disposes the previous one.
final mediaDiskCacheProvider = Provider<MediaDiskCache?>((ref) {
  final root = ref.watch(mediaCacheRootProvider).value;
  final (communityId, signer) = ref.watch(
    activeCommunityProvider.select((value) {
      final community = value.value;
      if (community == null) return (null, null);
      final signer = pubkeyFromNsec(community.nsec?.trim()) ?? community.pubkey;
      return (community.id, signer);
    }),
  );
  if (root == null || communityId == null || signer == null) return null;
  final cache = MediaDiskCache.open(
    MediaDiskCache.directoryFor(root, communityId, signer.toLowerCase()),
  );
  ref.onDispose(() => unawaited(cache.dispose()));
  return cache;
});
