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
import 'package:uuid/uuid.dart';

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
/// [mediaCacheRootProvider]), deleted with its community. Least recently used
/// files are evicted once the directory holds more than [maxBytes].
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
  /// Tests can shrink the budgets, the delay after a write or first read
  /// before eviction runs (10 s in `flutter_cache_manager`), and how long
  /// requests wait for the directory or for a previous cache to close.
  MediaDiskCache.open(
    FutureOr<Directory?> directory, {
    @visibleForTesting DateTime Function() now = DateTime.now,
    @visibleForTesting int byteBudget = maxBytes,
    @visibleForTesting int fileBudget = maxFiles,
    @visibleForTesting Duration? evictionDelay,
    @visibleForTesting int evictionsPerPass = maxEvictionsPerPass,
    @visibleForTesting Duration openTimeout = _openTimeout,
  }) : _now = now {
    CacheManager? openNow(Directory directory) {
      if (isClosed) return null;
      _path = directory.path;
      final (manager, index) = _openManager(
        directory,
        byteBudget: byteBudget,
        fileBudget: fileBudget,
        evictionDelay: evictionDelay,
        evictionsPerPass: evictionsPerPass,
      );
      _index = index;
      return manager;
    }

    _manager = Future.value(directory)
        // Images wait for the disk, so never wait on it for long.
        .timeout(openTimeout, onTimeout: () => null)
        .then((directory) async {
          if (directory == null || isClosed) return null;
          // A cache on this directory that is still closing, after a switch
          // away and back, may hold saves its index has not recorded yet.
          // Opening after it keeps this one's sweep from deleting them, and
          // keeps its final flush from overwriting this one's index.
          final previous = _closing[directory.path];
          if (previous == null) return openNow(directory);
          var closed = true;
          await previous.timeout(
            openTimeout,
            onTimeout: () {
              closed = false;
            },
          );
          if (closed) return openNow(directory);
          // Requests waiting now go without the disk. Later ones get it
          // once the previous cache has closed.
          unawaited(
            previous
                .then((_) {
                  if (isClosed) return;
                  final manager = openNow(directory);
                  if (manager != null) _manager = Future.value(manager);
                })
                .catchError((Object error) {
                  debugPrint('[MediaDiskCache] open failed: $error');
                }),
          );
          return null;
        })
        .catchError((Object error) {
          debugPrint('[MediaDiskCache] open failed: $error');
          return null;
        });
  }

  static (CacheManager, MediaCacheIndex) _openManager(
    Directory directory, {
    required int byteBudget,
    required int fileBudget,
    required Duration? evictionDelay,
    required int evictionsPerPass,
  }) {
    final index = MediaCacheIndex(
      File(p.join(directory.path, 'index.json')),
      directory,
      byteBudget,
      evictionsPerPass: evictionsPerPass,
    );
    final manager = CacheManager(
      Config(
        directory.path,
        // Age never evicts: the byte budget below does.
        stalePeriod: _neverStale,
        maxNrOfCacheObjects: fileBudget,
        repo: index,
        fileSystem: _DirectoryFileSystem(directory),
      ),
    );
    if (evictionDelay != null) {
      manager.store.cleanupRunMinInterval = evictionDelay;
    }
    // The package schedules a pass only on an index lookup, so a pass that
    // stops at its limit looks up a key to schedule the next.
    index.onMoreToEvict = () => unawaited(
      manager.store.retrieveCacheData(_nextPassKey, ignoreMemCache: true),
    );
    return (manager, index);
  }

  static const _nextPassKey = 'media-disk-cache:next-eviction-pass';

  static const _openTimeout = Duration(seconds: 3);

  /// Most bytes one community's cache keeps on disk.
  static const maxBytes = 250 * 1024 * 1024;

  /// Most files one community's cache keeps, so many tiny files (emoji,
  /// avatars) cannot cost far more disk and index work than their bytes.
  static const maxFiles = 10000;

  /// Most files one eviction pass deletes. Each costs UI-isolate time, so a
  /// large image saved into a cache full of small ones evicts them over
  /// several passes rather than in one long pause.
  static const maxEvictionsPerPass = 256;

  /// Age at which a non-relay image is refetched in the background.
  static const externalRefreshAge = Duration(days: 7);

  static const _neverStale = Duration(days: 365 * 100);

  /// Replaced once, when a previous cache on this directory outlasted the
  /// open wait and has since closed.
  late Future<CacheManager?> _manager;
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

  /// Deletes every directory under [root] except the [retained] communities'
  /// (by id), including writes that landed after their removal. A failed
  /// delete is retried on the next call.
  ///
  /// [retained] is read again before each delete, so a community added while
  /// the sweep runs is kept.
  static Future<void> removeOrphans(
    Directory root,
    Set<String> Function() retained,
  ) => _deleteExcept(
    Directory(p.join(root.path, 'communities')),
    () => {for (final id in retained()) communityDirectory(root, id).path},
  );

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
    if (isClosed) return;
    try {
      await (await _manager)?.removeFile(url);
    } on Object catch (error) {
      _logFailure(error);
    }
  }

  /// Whether [url] has saved bytes, from the index alone, without reading
  /// them.
  Future<bool> contains(String url) async {
    if (isClosed) return false;
    try {
      final manager = await _manager;
      return await manager?.getFileFromCache(url, ignoreMemCache: true) != null;
    } on Object catch (error) {
      _logFailure(error);
      return false;
    }
  }

  /// [url]'s saved bytes, or null when they are not on disk.
  Future<MediaDiskCacheHit?> read(String url) async {
    final FileInfo? info;
    final Uint8List bytes;
    final manager = await _manager;
    if (manager == null || isClosed) return null;
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
  /// refetched; others are refetched after [externalRefreshAge]. Failures are
  /// logged, never thrown: a missing entry only costs a later download.
  Future<void> write(String url, Uint8List bytes, {required bool immutable}) {
    // Overlapping saves of one URL would each create a file, and the index
    // tracks only the last, so the other would stay on disk past eviction.
    // Each save waits for the one before it, then replaces its file.
    final saved = (_writes[url] ?? Future.value()).then(
      (_) => _write(url, bytes, immutable: immutable),
    );
    _writes[url] = saved;
    return saved.whenComplete(() {
      if (identical(_writes[url], saved)) _writes.remove(url);
    });
  }

  final _writes = <String, Future<void>>{};

  Future<void> _write(
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
      final store = manager.store;
      // Like `putFile`, keep the URL's path, but publish the bytes with a
      // rename so a reader sees the old file or the new one, never a part.
      final previous = await store.retrieveCacheData(url);
      final saved =
          (previous ??
                  CacheObject(
                    url,
                    relativePath: '${const Uuid().v1()}.file',
                    validTill: _now(),
                  ))
              .copyWith(
                validTill: _now().add(
                  immutable ? _neverStale : externalRefreshAge,
                ),
              );
      await (store.fileSystem as _DirectoryFileSystem).replace(
        saved.relativePath,
        bytes,
      );
      await store.putFile(saved);
      // Replacing a URL the package already holds in memory schedules no
      // eviction, so a refresh that grows the cache would leave it over
      // budget. An index lookup schedules it.
      await manager.getFileFromCache(url, ignoreMemCache: true);
    } on Object catch (error) {
      _logFailure(error);
    }
  }

  /// Stops writes, lets saves in progress finish, then flushes the index.
  Future<void> dispose() {
    if (_closed.isCompleted) return _disposed;
    _closed.complete();
    // The package leaves an eviction pass scheduled, which could run after
    // another cache opened this directory and save a stale index over its.
    _index?.retire();
    final disposed = _disposed = _manager
        .then((manager) async {
          await Future.wait(_writes.values.toList());
          await manager?.dispose();
        })
        .catchError((Object error) {
          debugPrint('[MediaDiskCache] close failed: $error');
        });
    final path = _path;
    if (path != null) {
      _closing[path] = disposed;
      unawaited(
        disposed.whenComplete(() {
          if (identical(_closing[path], disposed)) _closing.remove(path);
        }),
      );
    }
    return disposed;
  }

  Future<void> _disposed = Future.value();

  /// The directory this cache opened, once it has.
  String? _path;

  MediaCacheIndex? _index;

  /// Caches still closing, by directory.
  static final _closing = <String, Future<void>>{};

  static void _logFailure(Object error) =>
      debugPrint('[MediaDiskCache] write failed: $error');
}

/// `flutter_cache_manager` caps a cache only by file count. This index keeps
/// its JSON format but evicts by bytes: once the files total more than
/// [_maxBytes], the least recently used are over capacity.
@visibleForTesting
class MediaCacheIndex extends JsonCacheInfoRepository {
  MediaCacheIndex(
    super.file,
    this._directory,
    this._maxBytes, {
    this.evictionsPerPass = MediaDiskCache.maxEvictionsPerPass,
  }) : super.withFile();

  final Directory _directory;
  final int _maxBytes;

  /// Most files [getObjectsOverCapacity] returns at once.
  final int evictionsPerPass;

  /// Called when more files are over capacity than one pass returns.
  void Function()? onMoreToEvict;

  bool _retired = false;

  /// Stops eviction for good, once its cache is disposed: a pass still
  /// scheduled finds nothing to evict, and one in progress records nothing.
  /// Saves still finishing are recorded.
  void retire() => _retired = true;

  /// Set once closing starts. A later change, such as a read marking a file
  /// used after the cache was disposed, would save this index over one a
  /// newer cache wrote, so it is dropped.
  bool _closed = false;

  /// Deletes files the index doesn't track, once, after it opens: a save
  /// interrupted by a full disk or the app exiting, or one whose index entry
  /// was not yet saved. Eviction sees only indexed files. The sweep runs in
  /// the background, since listing a full directory would delay the first
  /// image.
  @override
  Future<bool> open() async {
    final opened = await super.open();
    _sweep ??= _deleteUntrackedFiles();
    return opened;
  }

  Future<void>? _sweep;

  /// Waits for the sweep, so a cache opening this directory next starts
  /// after it.
  @override
  Future<bool> close() async {
    await _sweep;
    _closed = true;
    return super.close();
  }

  /// How far a file's modification time may trail the clock, on file
  /// systems that record it coarsely.
  static const _mtimeSlack = Duration(seconds: 5);

  Future<void> _deleteUntrackedFiles() async {
    // A save that starts while this runs is newer, so it is kept.
    final cutoff = DateTime.now().subtract(_mtimeSlack);
    final Set<String> tracked;
    final List<FileSystemEntity> entries;
    try {
      if (!await _directory.exists()) return;
      tracked = {for (final o in await getAllObjects()) o.relativePath};
      entries = await _directory.list().toList();
    } on FileSystemException catch (error) {
      debugPrint('[MediaDiskCache] sweep failed: $error');
      return;
    }
    for (final entry in entries) {
      final name = p.basename(entry.path);
      if (entry is! File ||
          name.startsWith('index.json') ||
          tracked.contains(name)) {
        continue;
      }
      try {
        if ((await entry.lastModified()).isBefore(cutoff)) {
          await entry.delete();
        }
      } on FileSystemException {
        // Renamed or deleted since the listing.
      }
    }
  }

  /// The least recently used files past [capacity] files or past the byte
  /// budget, whichever keeps fewer, at most [evictionsPerPass] of them.
  @override
  Future<List<CacheObject>> getObjectsOverCapacity(int capacity) async {
    if (_retired) return [];
    final newestFirst = await getAllObjects()
      ..sort((a, b) => b.touched!.compareTo(a.touched!));
    var total = 0;
    var count = 0;
    final over = [
      for (final object in newestFirst)
        if ((total += object.length ?? 0) > _maxBytes || ++count > capacity)
          object,
    ];
    if (over.length <= evictionsPerPass) return over;
    onMoreToEvict?.call();
    return over.sublist(over.length - evictionsPerPass);
  }

  @override
  Future<int> deleteAll(Iterable<int> ids) async =>
      _retired || _closed ? 0 : super.deleteAll(ids);

  @override
  Future<int> delete(int id) async => _closed ? 0 : super.delete(id);

  /// `putFile` records no length, and replacing a file's bytes keeps the old
  /// one, so measure the file on every write.
  @override
  Future<dynamic> updateOrInsert(CacheObject cacheObject) async {
    if (_closed) return cacheObject;
    final file = File(p.join(_directory.path, cacheObject.relativePath));
    final length = await file.exists() ? await file.length() : 0;
    if (_closed) return cacheObject;
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

  /// Writes [bytes] beside [name], then renames them over it, so readers
  /// never see a partial file. A failed write leaves no file behind.
  Future<void> replace(String name, Uint8List bytes) async {
    // A name of its own, so a sweep that found an earlier save's abandoned
    // part cannot delete this one.
    final part = await createFile('$name.${const Uuid().v4()}.part');
    try {
      await part.writeAsBytes(bytes);
      await part.rename(_directory.childFile(name).path);
    } on Object {
      try {
        await part.delete();
      } on FileSystemException {
        // Never created.
      }
      rethrow;
    }
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
