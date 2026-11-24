import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:isolate';

import 'package:crypto/crypto.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:path/path.dart' as p;
import 'package:path_provider/path_provider.dart';
import 'package:sqlite3/sqlite3.dart';

import '../community/community_provider.dart';
import '../diagnostics/diagnostic_reporter.dart';
import '../relay/nostr_models.dart';
import '../relay/relay_provider.dart' show pubkeyFromNsec;
import 'saved_message_verification.dart';

part 'saved_message_store_disk.dart';

/// The local community and signing identity that own a saved page.
typedef SavedMessageScope = ({String communityId, String pubkey});

/// A page address. The root is empty for the channel timeline.
typedef SavedMessagePage = ({String channelId, String rootId});

/// The cache scope follows the signing identity of the active community.
final savedMessageScopeProvider = Provider<SavedMessageScope?>((ref) {
  final active = ref.watch(activeCommunityProvider).value;
  if (active == null) return null;
  // The active selection can be refreshing while the current list has
  // already dropped or replaced its identity.
  final retained = ref.watch(
    communityListProvider.select(
      (result) =>
          result.unwrapPrevious().value?.any(
            (community) =>
                community.id == active.id &&
                community.nsec == active.nsec &&
                community.pubkey == active.pubkey,
          ) ??
          false,
    ),
  );
  if (!retained) return null;
  final pubkey = pubkeyFromNsec(active.nsec?.trim()) ?? active.pubkey;
  return pubkey == null ? null : (communityId: active.id, pubkey: pubkey);
});

/// The relay signing key saved for the active identity, read once per scope.
/// It lets a cold, offline launch recognize relay-signed messages.
final savedRelayAuthorProvider = FutureProvider<String?>((ref) async {
  final scope = ref.watch(savedMessageScopeProvider);
  if (scope == null) return null;
  return ref.read(savedMessageStoreProvider).relayAuthor(scope);
});

/// Captures the right to read or write a page before an asynchronous load.
final class SavedMessageTicket {
  const SavedMessageTicket._(
    this._scope,
    this._page,
    this._scopeEpoch,
    this._channelEpoch,
  );

  final String _scope;
  final SavedMessagePage _page;
  final int _scopeEpoch;
  final int _channelEpoch;
}

/// The app's private cache directory on iOS and Android.
final savedMessageRootProvider = FutureProvider<Directory?>(
  (ref) async => Platform.isIOS || Platform.isAndroid
      ? getApplicationCacheDirectory()
      : null,
);

/// A single serial queue orders cache operations and privacy deletions.
final savedMessageStoreProvider = Provider<SavedMessageStore>((ref) {
  return SavedMessageStore(
    root: () => ref.read(savedMessageRootProvider.future),
    diagnostics: ref.read(diagnosticReporterProvider),
    purgeRoot: Platform.isIOS || Platform.isAndroid
        ? getApplicationSupportDirectory
        : null,
  );
});

/// A bounded, disposable store of previously opened conversations.
final class SavedMessageStore {
  static const maxPageEvents = 2101; // 50 rows, summaries and two aux hops.

  SavedMessageStore({
    required Future<Directory?> Function() root,
    required DiagnosticReporter diagnostics,
    Future<Directory> Function()? purgeRoot,
    this.payloadBudget = 64 * 1024 * 1024,
    this.databasePageLimit = _maxDatabasePages,
  }) : _root = root,
       _purgeRoot = purgeRoot,
       _diagnostics = diagnostics;

  final Future<Directory?> Function() _root;
  final Future<Directory> Function()? _purgeRoot;
  final DiagnosticReporter _diagnostics;
  final int payloadBudget;
  final int databasePageLimit;
  final _scopeEpochs = <String, int>{};
  final _channelEpochs = <(String, String), int>{};
  final _pendingEventRemovals = <(String, String), Map<String, bool>>{};
  final _recentMutations = <(String, String, String), bool>{};
  Future<void> _tail = Future.value();
  Future<void> _purgeMarkerTail = Future.value();
  int _activePurgeMarkers = 0;
  // Read the marker once per app session. This store owns all its mutations.
  bool? _needsReset;
  Directory? _purgeDirectory;

  SavedMessageTicket capture(SavedMessageScope scope, SavedMessagePage page) {
    final key = _scopeKey(scope);
    return SavedMessageTicket._(
      key,
      page,
      _scopeEpochs.putIfAbsent(key, () => 0),
      _channelEpochs[(key, page.channelId)] ?? 0,
    );
  }

  bool isCurrent(SavedMessageTicket ticket) =>
      (_scopeEpochs[ticket._scope] ?? 0) == ticket._scopeEpoch &&
      (_channelEpochs[(ticket._scope, ticket._page.channelId)] ?? 0) ==
          ticket._channelEpoch;

  /// Live edits and deletions can be newer than a subsequent replica read.
  ({Set<String> edited, Set<String> deleted}) threadMutations(
    SavedMessageTicket ticket,
    Iterable<String> ids,
  ) {
    final edited = <String>{};
    final deleted = <String>{};
    for (final id in ids) {
      final mutation =
          _recentMutations[(ticket._scope, ticket._page.channelId, id)];
      if (mutation != null) (mutation ? edited : deleted).add(id);
    }
    return (edited: edited, deleted: deleted);
  }

  /// Reads a verified page and records that the conversation was opened.
  Future<List<NostrEvent>?> read(SavedMessageTicket ticket) async {
    final result = await _forPage(ticket, _DiskAction.read);
    return isCurrent(ticket) ? result?.events : null;
  }

  /// Reads trusted events and the targets removed from this thread page.
  Future<({List<NostrEvent> events, Map<String, bool> pruned})?> inspectThread(
    SavedMessageTicket ticket,
  ) async {
    final result = await _forPage(ticket, _DiskAction.inspect);
    final events = result?.events;
    if (events == null || !isCurrent(ticket)) return null;
    return (events: events, pruned: result!.pruned);
  }

  /// Marks a warm open without reading or decoding the saved page.
  Future<void> touch(SavedMessageTicket ticket) async {
    await _forPage(ticket, _DiskAction.touch);
  }

  /// Replaces a page using original signed events from the relay.
  Future<void> save(
    SavedMessageTicket ticket,
    List<NostrEvent> events, {
    required String? relayAuthor,
  }) async {
    await _forPage(
      ticket,
      _DiskAction.save,
      events: events,
      relayAuthor: relayAuthor,
    );
  }

  /// The relay signing key last saved with this identity's pages: the key
  /// that verified them, replaced when a save finds the relay rotated it.
  /// Null when none was saved, the cache awaits a purge, or [scope] is
  /// removed meanwhile.
  Future<String?> relayAuthor(SavedMessageScope scope) async {
    final key = _scopeKey(scope);
    final epoch = _scopeEpochs.putIfAbsent(key, () => 0);
    final author = await _enqueue((directory) async {
      final marker = (await _purgeMarker(directory))!;
      final needsReset = await _withPurgeMarker(
        () async => _needsReset ??= await _purgePending(marker),
      );
      if (needsReset || _scopeEpochs[key] != epoch) return null;
      final result = await _runDisk(
        _DiskRequest(
          path: _path(directory, key),
          action: _DiskAction.relayAuthor,
          page: (channelId: '', rootId: ''),
          databasePageLimit: databasePageLimit,
        ),
      );
      if (result.reset) _reportReset();
      return result.relayAuthor;
    });
    return _scopeEpochs[key] == epoch &&
            author != null &&
            _lowerHex64.hasMatch(author)
        ? author
        : null;
  }

  /// Retires all in-flight work and deletes every version for this identity.
  Future<void> removeScope(SavedMessageScope scope) {
    final key = _scopeKey(scope);
    _scopeEpochs.update(key, (epoch) => epoch + 1, ifAbsent: () => 1);
    _channelEpochs.removeWhere((entry, _) => entry.$1 == key);
    _pendingEventRemovals.removeWhere((entry, _) => entry.$1 == key);
    _recentMutations.removeWhere((entry, _) => entry.$1 == key);
    return _purge((directory) => _sweep(directory, removedScope: key));
  }

  /// Retires every page of a channel that was deleted, left or denied.
  /// With [deferDisk], returns once the durable marker is written.
  Future<void> removeChannel(
    SavedMessageScope scope,
    String channelId, {
    bool deferDisk = false,
  }) {
    final key = _scopeKey(scope);
    _retireChannel((key, channelId));
    _pendingEventRemovals.remove((key, channelId));
    _recentMutations.removeWhere(
      (entry, _) => entry.$1 == key && entry.$2 == channelId,
    );
    return _purge(
      (directory) => _removeFromDisk(directory, key, channelId),
      deferDisk: deferDisk,
    );
  }

  /// Drops changed events and keeps the rest of the saved conversation.
  void removeChannelEvents(
    SavedMessageScope scope,
    String channelId,
    Set<String> targets, {
    bool edited = false,
  }) {
    if (targets.isEmpty) return;
    final key = (_scopeKey(scope), channelId);
    _retireChannel(key);
    final pending = _pendingEventRemovals[key];
    final batch = pending ?? <String, bool>{};
    for (final target in targets) {
      if (batch.length > maxPageEvents) break;
      batch.update(
        target,
        (wasEdit) => wasEdit && edited,
        ifAbsent: () => edited,
      );
      final targetKey = (key.$1, channelId, target);
      final previous = _recentMutations.remove(targetKey);
      _recentMutations[targetKey] = previous == false ? false : edited;
      if (_recentMutations.length > maxPageEvents) {
        _recentMutations.remove(_recentMutations.keys.first);
      }
    }
    if (pending != null) return;
    _pendingEventRemovals[key] = batch;
    unawaited(
      _purge(
        (directory) => _removeFromDisk(
          directory,
          key.$1,
          key.$2,
          targets: batch.length > maxPageEvents ? null : batch,
        ),
        shouldRun: () {
          if (!identical(_pendingEventRemovals[key], batch)) return false;
          _pendingEventRemovals.remove(key);
          return true;
        },
      ).whenComplete(() {
        if (identical(_pendingEventRemovals[key], batch)) {
          _pendingEventRemovals.remove(key);
        }
      }),
    );
  }

  void _retireChannel((String, String) key) {
    _channelEpochs.update(key, (epoch) => epoch + 1, ifAbsent: () => 1);
  }

  // One durable marker covers the serial purge queue. If the app stops
  // before clearing it, we discard the cache before publishing any page.
  Future<void> _purge(
    Future<void> Function(String directory) remove, {
    bool Function()? shouldRun,
    bool deferDisk = false,
  }) async {
    final ready = deferDisk ? _reservePurgeMarker() : null;
    var entered = false;
    final completed =
        _enqueue((directory) async {
          entered = true;
          if (shouldRun?.call() == false) return;
          final marker = await (ready ?? _reservePurgeMarker(directory));
          var removed = false;
          try {
            if (_needsReset ?? false) {
              await _sweep(directory, retained: const {});
            } else {
              await remove(directory);
            }
            _needsReset = false;
            removed = true;
          } finally {
            if (marker != null) {
              await _releasePurgeMarker(marker, removed: removed);
            }
          }
        }, onFailure: () => _needsReset = true).then((_) async {
          // The cache root can fail before the queued operation enters.
          if (!entered && ready != null) {
            final marker = await ready;
            if (marker != null) {
              await _releasePurgeMarker(marker, removed: false);
            }
          }
        });
    // A confirmed action may return once a cold launch can detect the purge.
    // If recording fails, still wait for the queued removal attempt.
    if (ready == null || await ready == null) await completed;
  }

  Future<T> _withPurgeMarker<T>(Future<T> Function() action) {
    final next = _purgeMarkerTail.then((_) => action());
    _purgeMarkerTail = next.then<void>((_) {}, onError: (Object _) {});
    return next;
  }

  Future<File?> _reservePurgeMarker([String? cacheDirectory]) =>
      _withPurgeMarker(() async {
        try {
          final marker = await _purgeMarker(cacheDirectory);
          if (marker == null) return null;
          if (_needsReset == null) {
            final interrupted = await _purgePending(marker);
            _needsReset ??= interrupted;
          }
          if (_activePurgeMarkers == 0) {
            await marker.parent.create(recursive: true);
            await marker.writeAsString('pending', flush: true);
          }
          _activePurgeMarkers++;
          return marker;
        } catch (error, stackTrace) {
          _needsReset = true;
          _report(error, stackTrace);
          return null;
        }
      });

  Future<void> _releasePurgeMarker(File marker, {required bool removed}) =>
      _withPurgeMarker(() async {
        _activePurgeMarkers--;
        if (!removed) _needsReset = true;
        if (_activePurgeMarkers == 0 && _needsReset == false) {
          await _clearPurgeMarker(marker);
        }
      });

  Future<File?> _purgeMarker(String? directory) async {
    final root = _purgeDirectory ??= await _purgeRoot?.call();
    final base =
        root?.path ??
        (directory == null ? (await _root())?.path : p.dirname(directory));
    return base == null ? null : File(p.join(base, 'saved_message_purge'));
  }

  Future<bool> _purgePending(File marker) async {
    try {
      final file = await marker.open();
      await file.close();
      return true;
    } on FileSystemException catch (error) {
      if (error.osError?.errorCode == _noSuchFile) return false;
      rethrow;
    }
  }

  Future<void> _clearPurgeMarker(File marker) async {
    try {
      await marker.delete();
    } on FileSystemException catch (error) {
      if (error.osError?.errorCode != _noSuchFile) rethrow;
    }
  }

  Future<void> _removeFromDisk(
    String directory,
    String key,
    String channelId, {
    Map<String, bool>? targets,
  }) async {
    try {
      final result = await _runDisk(
        _DiskRequest(
          path: _path(directory, key),
          action: _DiskAction.removeChannel,
          page: (channelId: channelId, rootId: ''),
          targets: targets,
          payloadBudget: payloadBudget,
          databasePageLimit: databasePageLimit,
        ),
      );
      if (result.reset) _reportReset();
    } catch (error, stackTrace) {
      _report(error, stackTrace);
      // If SQL cannot remove that channel's data, discard the disposable scope.
      await _sweep(directory, removedScope: key);
    }
  }

  /// Removes retired identity databases from interrupted account cleanup and
  /// incompatible cache formats left by an app update.
  Future<void> removeOrphans(Set<SavedMessageScope> retained) {
    final keys = retained.map(_scopeKey).toSet();
    // A channel may have started a new load while account removal awaited
    // storage. Fence those tickets as well as deleting the files.
    _scopeEpochs.updateAll(
      (key, epoch) => keys.contains(key) ? epoch : epoch + 1,
    );
    _recentMutations.removeWhere((entry, _) => !keys.contains(entry.$1));
    return _enqueue((directory) => _sweep(directory, retained: keys));
  }

  Future<_DiskResult?> _forPage(
    SavedMessageTicket ticket,
    _DiskAction action, {
    List<NostrEvent> events = const [],
    String? relayAuthor,
  }) => _enqueue<_DiskResult?>((directory) async {
    if (!isCurrent(ticket)) return null;
    final marker = (await _purgeMarker(directory))!;
    final needsReset = await _withPurgeMarker(
      () async => _needsReset ??= await _purgePending(marker),
    );
    if (needsReset) {
      if (action != _DiskAction.save) return null;
      await _sweep(directory, retained: const {});
      await _withPurgeMarker(() async {
        if (_activePurgeMarkers == 0) await _clearPurgeMarker(marker);
        _needsReset = false;
      });
    }
    if (!isCurrent(ticket)) return null;
    final result = await _runDisk(
      _DiskRequest(
        path: _path(directory, ticket._scope),
        action: action,
        page: ticket._page,
        events: events,
        relayAuthor: relayAuthor,
        requiredEdits: ticket._page.rootId.isEmpty
            ? const {}
            : threadMutations(ticket, events.map((event) => event.id)).edited,
        payloadBudget: payloadBudget,
        databasePageLimit: databasePageLimit,
      ),
    );
    if (result.reset) _reportReset();
    if (result.recencyFailure case final stackTrace?) {
      _diagnostics.reportError(
        const FormatException('Saved message recency update failed'),
        stackTrace,
        operation: DiagnosticOperation.storedData,
      );
    }
    return result;
  });

  Future<T?> _enqueue<T>(
    Future<T> Function(String directory) operation, {
    void Function()? onFailure,
  }) {
    final next = _tail.then((_) async {
      try {
        final root = await _root();
        if (root == null) return null;
        return await operation(p.join(root.path, 'saved_messages'));
      } catch (error, stackTrace) {
        onFailure?.call();
        _report(error, stackTrace);
        return null;
      }
    });
    _tail = next.then<void>((_) {});
    return next;
  }

  Future<void> _sweep(
    String directory, {
    Set<String>? retained,
    String? removedScope,
  }) async {
    final obsolete = await _sweepOnWorker(directory, retained, removedScope);
    if (obsolete) _reportReset();
  }

  void _reportReset() => _report(
    const FormatException('Saved message database was discarded'),
    StackTrace.current,
  );

  // Exceptions from filesystem or SQL APIs can contain a path or event data.
  void _report(Object error, StackTrace stackTrace) {
    _diagnostics.reportError(
      FormatException('Saved message storage failed (${error.runtimeType})'),
      stackTrace,
      operation: DiagnosticOperation.storedData,
    );
  }
}

// POSIX ENOENT on iOS and Android. Other errors leave the cache unavailable.
const _noSuchFile = 2;

final _lowerHex64 = RegExp(r'^[0-9a-f]{64}$');

String _scopeKey(SavedMessageScope scope) => sha256
    .convert(utf8.encode('${scope.communityId}\n${scope.pubkey.toLowerCase()}'))
    .toString();
