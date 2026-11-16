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
  /// Opens the store in [directory], creating it on first write, or a store
  /// that saves nothing when [directory] is null. Reads and writes wait for
  /// [directory] to resolve, so a load never skips the disk while it does.
  /// [now] is the clock staleness is judged by.
  ///
  /// Tests can shrink the budgets and the delay after a write or first read
  /// before eviction runs (10 s in `flutter_cache_manager`).
  MediaDiskCache.open(
    FutureOr<Directory?> directory, {
    @visibleForTesting DateTime Function() now = DateTime.now,
    @visibleForTesting int byteBudget = maxBytes,
    @visibleForTesting int fileBudget = maxFiles,
    @visibleForTesting Duration? evictionDelay,
  }) : _now = now {
    _manager = Future.value(directory)
        // Images wait for the disk, so never wait on it for long.
        .timeout(_openTimeout, onTimeout: () => null)
        .then((directory) {
          if (directory == null || isClosed) return null;
          return _openManager(
            directory,
            byteBudget: byteBudget,
            fileBudget: fileBudget,
            evictionDelay: evictionDelay,
          );
        })
        .catchError((Object error) {
          debugPrint('[MediaDiskCache] open failed: $error');
          return null;
        });
  }

  static CacheManager _openManager(
    Directory directory, {
    required int byteBudget,
    required int fileBudget,
    required Duration? evictionDelay,
  }) {
    final manager = CacheManager(
      Config(
        directory.path,
        // Age never evicts: the byte budget below does.
        stalePeriod: _neverStale,
        maxNrOfCacheObjects: fileBudget,
        repo: MediaCacheIndex(
          File(p.join(directory.path, 'index.json')),
          directory,
          byteBudget,
        ),
        fileSystem: _DirectoryFileSystem(directory),
      ),
    );
    if (evictionDelay != null) {
      manager.store.cleanupRunMinInterval = evictionDelay;
    }
    return manager;
  }

  static const _openTimeout = Duration(seconds: 3);

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

  late final Future<CacheManager?> _manager;
  final DateTime Function() _now;
  final _closed = Completer<void>();

  /// Completes once the cache is disposed, when its community or account is
  /// retired.
  Future<void> get closed => _closed.future;

  /// Whether the cache was disposed.
  bool get isClosed => _closed.isCompleted;

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

  /// Keeps only each retained community's cache for its current signer
  /// (community id to signer pubkey) and deletes every other directory under
  /// [root]: removed communities and replaced accounts, including writes
  /// that landed after their removal. A failed delete is retried on the
  /// next call.
  ///
  /// [retained] is read again before each delete, so a community or account
  /// added while the sweep runs is kept.
  static Future<void> removeOrphans(
    Directory root,
    Map<String, String?> Function() retained,
  ) async {
    Set<String> keep() => {
      for (final MapEntry(key: id, value: signer) in retained().entries)
        if (signer != null) directoryFor(root, id, signer).path,
    };
    final communities = Directory(p.join(root.path, 'communities'));
    await _deleteExcept(communities, () {
      return {for (final path in keep()) p.dirname(p.dirname(path))};
    });
    if (!await communities.exists()) return;
    await for (final community in communities.list(followLinks: false)) {
      if (community is! Directory) continue;
      await _deleteExcept(Directory(p.join(community.path, 'media')), keep);
    }
  }

  /// Deletes the entries of [parent] whose paths are not in [keep].
  static Future<void> _deleteExcept(
    Directory parent,
    Set<String> Function() keep,
  ) async {
    if (!await parent.exists()) return;
    await for (final entry in parent.list(followLinks: false)) {
      if (keep().contains(entry.path)) continue;
      try {
        await entry.delete(recursive: true);
      } on FileSystemException catch (error) {
        debugPrint('[MediaDiskCache] orphan cleanup failed: $error');
      }
    }
  }

  /// Removes [url]'s saved bytes, for example when they no longer decode.
  /// Failures are logged; a later read finds the same bad bytes.
  Future<void> remove(String url) async {
    try {
      await (await _manager)?.removeFile(url);
    } on Object catch (error) {
      _logFailure(error);
    }
  }

  /// [url]'s saved bytes, or null when they are not on disk.
  Future<MediaDiskCacheHit?> read(String url) async {
    final FileInfo? info;
    final Uint8List bytes;
    final manager = await _manager;
    if (manager == null) return null;
    try {
      // Past the first lookup, the package serves a URL from memory without
      // marking it used, so a file read often would be evicted first. Going
      // to the index marks it used, and schedules eviction.
      info = await manager.getFileFromCache(url, ignoreMemCache: true);
      if (info == null) return null;
      bytes = await info.file.readAsBytes();
    } on FileSystemException {
      // The OS may purge cache files under the index. Drop the entry and
      // load from the network.
      unawaited(remove(url));
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
    if (isClosed || bytes.isEmpty) return;
    final manager = await _manager;
    if (manager == null || isClosed) return;
    try {
      if (bytes.length > maxFileBytes) {
        await manager.removeFile(url);
        return;
      }
      await manager.putFile(
        url,
        bytes,
        maxAge: immutable ? _neverStale : externalRefreshAge,
      );
      // Replacing a URL the package already holds in memory schedules no
      // eviction, so a refresh that grows the cache would leave it over
      // budget. An index lookup schedules it.
      await manager.getFileFromCache(url, ignoreMemCache: true);
    } on Object catch (error) {
      _logFailure(error);
    }
  }

  /// Stops writes, then flushes the index.
  Future<void> dispose() {
    if (_closed.isCompleted) return _disposed;
    _closed.complete();
    return _disposed = _manager
        .then((manager) => manager?.dispose())
        .catchError((Object error) {
          debugPrint('[MediaDiskCache] close failed: $error');
        });
  }

  Future<void> _disposed = Future.value();

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
  final (communityId, signer) = ref.watch(
    activeCommunityProvider.select((value) {
      final community = value.value;
      if (community == null) return (null, null);
      final signer = pubkeyFromNsec(community.nsec?.trim()) ?? community.pubkey;
      return (community.id, signer);
    }),
  );
  if (communityId == null || signer == null) return null;
  // The cache exists at once and opens its directory when the root
  // resolves, so images shown during startup still read the disk.
  final cache = MediaDiskCache.open(
    ref
        .watch(mediaCacheRootProvider.future)
        .then(
          (root) => root == null
              ? null
              : MediaDiskCache.directoryFor(
                  root,
                  communityId,
                  signer.toLowerCase(),
                ),
        ),
  );
  ref.onDispose(() => unawaited(cache.dispose()));
  return cache;
});
