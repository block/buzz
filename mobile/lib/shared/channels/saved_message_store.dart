import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:isolate';

import 'package:crypto/crypto.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:path/path.dart' as p;
import 'package:sqlite3/sqlite3.dart';

import '../community/community_provider.dart';
import '../diagnostics/diagnostic_reporter.dart';
import '../relay/media_disk_cache.dart';
import '../relay/nostr_models.dart';
import '../relay/relay_provider.dart' show pubkeyFromNsec;
import 'saved_message_cache_lifecycle.dart';
import 'saved_message_verification.dart';

part 'saved_message_store_disk.dart';

/// The local community and signing identity that own a saved page.
typedef SavedMessageScope = ({String communityId, String pubkey});

/// A page address. The root is empty for the channel timeline.
typedef SavedMessagePage = ({String channelId, String rootId});

/// The cache scope follows the signing identity of the active community.
final savedMessageScopeProvider = Provider<SavedMessageScope?>((ref) {
  return ref.watch(
    activeCommunityProvider.select((result) {
      final community = result.value;
      if (community == null) return null;
      final pubkey = pubkeyFromNsec(community.nsec?.trim()) ?? community.pubkey;
      return pubkey == null
          ? null
          : (communityId: community.id, pubkey: pubkey);
    }),
  );
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

/// The app group's cache root on iOS. Override this in isolated tests.
final savedMessageRootProvider = FutureProvider<Directory?>(
  (ref) => Platform.isIOS ? ref.watch(mediaCacheRootProvider.future) : null,
);

/// A single serial queue orders cache operations and privacy deletions.
final savedMessageStoreProvider = Provider<SavedMessageStore>((ref) {
  final store = SavedMessageStore(
    root: () => ref.read(savedMessageRootProvider.future),
    diagnostics: ref.read(diagnosticReporterProvider),
  );
  ref.onDispose(store.dispose);
  return store;
});

/// A bounded, disposable store of previously opened conversations.
final class SavedMessageStore {
  SavedMessageStore({
    required Future<Directory?> Function() root,
    required DiagnosticReporter diagnostics,
    Future<SavedMessageCacheLease?> Function()? acquireLease,
    Stream<void>? foreground,
    this.payloadBudget = 64 * 1024 * 1024,
  }) : _root = root,
       _diagnostics = diagnostics,
       _acquireLease = acquireLease ?? SavedMessageCacheLifecycle.acquire,
       _foreground = foreground ?? SavedMessageCacheLifecycle.foreground;

  final Future<Directory?> Function() _root;
  final DiagnosticReporter _diagnostics;
  final Future<SavedMessageCacheLease?> Function() _acquireLease;
  final Stream<void> _foreground;
  final int payloadBudget;
  final _scopeEpochs = <String, int>{};
  final _channelEpochs = <(String, String), int>{};
  final _pendingEventRemovals = <(String, String), _PendingEventRemoval>{};
  StreamSubscription<void>? _foregroundSubscription;
  int _foregroundVersion = 0;
  Future<void> _tail = Future.value();

  SavedMessageTicket capture(SavedMessageScope scope, SavedMessagePage page) {
    final key = _scopeKey(scope);
    return SavedMessageTicket._(
      key,
      page,
      _scopeEpochs[key] ?? 0,
      _channelEpochs[(key, page.channelId)] ?? 0,
    );
  }

  bool isCurrent(SavedMessageTicket ticket) =>
      (_scopeEpochs[ticket._scope] ?? 0) == ticket._scopeEpoch &&
      (_channelEpochs[(ticket._scope, ticket._page.channelId)] ?? 0) ==
          ticket._channelEpoch;

  /// Reads a verified page and records that the conversation was opened.
  Future<List<NostrEvent>?> read(SavedMessageTicket ticket) async {
    final result = await _forPage(ticket, _DiskAction.read);
    return isCurrent(ticket) ? result?.events : null;
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

  /// Retires all in-flight work and deletes every version for this identity.
  Future<void> removeScope(SavedMessageScope scope) {
    final key = _scopeKey(scope);
    _scopeEpochs.update(key, (epoch) => epoch + 1, ifAbsent: () => 1);
    _channelEpochs.removeWhere((entry, _) => entry.$1 == key);
    _pendingEventRemovals.removeWhere((entry, _) => entry.$1 == key);
    return _enqueue((directory) => _sweep(directory, removedScope: key));
  }

  /// Retires every page of a channel that was deleted, left or denied.
  Future<void> removeChannel(SavedMessageScope scope, String channelId) {
    final key = _scopeKey(scope);
    _retireChannel((key, channelId));
    _pendingEventRemovals.remove((key, channelId));
    return _enqueue((directory) async {
      await _removeFromDisk(directory, key, channelId);
    });
  }

  /// Drops changed events and keeps the rest of the saved conversation.
  void removeChannelEvents(
    SavedMessageScope scope,
    String channelId,
    Set<String> targets,
  ) {
    if (targets.isEmpty) return;
    final key = (_scopeKey(scope), channelId);
    _retireChannel(key);
    _foregroundSubscription ??= _foreground.listen((_) {
      _foregroundVersion++;
      for (final entry in _pendingEventRemovals.entries) {
        if (entry.value.deferred) {
          entry.value.deferred = false;
          _scheduleEventRemoval(entry.key, entry.value);
        }
      }
    });
    final batch = _pendingEventRemovals.putIfAbsent(
      key,
      _PendingEventRemoval.new,
    );
    batch.addTargets(targets);
    _scheduleEventRemoval(key, batch);
  }

  void _scheduleEventRemoval((String, String) key, _PendingEventRemoval batch) {
    if (batch.queued || batch.deferred) return;
    batch.queued = true;
    var started = false;
    unawaited(
      _enqueue((directory) async {
        if (!identical(_pendingEventRemovals[key], batch)) return;
        started = true;
        final targets = batch.targets;
        batch.targets = {};
        final foregroundVersion = _foregroundVersion;
        final removed = await _removeFromDisk(
          directory,
          key.$1,
          key.$2,
          targets: targets.length > _maxWindowEvents ? null : targets,
          deferIfUnavailable: true,
        );
        if (!removed) {
          batch.addTargets(targets);
          batch.deferred = foregroundVersion == _foregroundVersion;
        }
      }).whenComplete(() {
        if (!identical(_pendingEventRemovals[key], batch)) return;
        batch.queued = false;
        if (!started || batch.targets.isEmpty) {
          _pendingEventRemovals.remove(key);
        } else {
          _scheduleEventRemoval(key, batch);
        }
      }),
    );
  }

  void dispose() {
    _pendingEventRemovals.clear();
    unawaited(_foregroundSubscription?.cancel());
  }

  void _retireChannel((String, String) key) {
    _channelEpochs.update(key, (epoch) => epoch + 1, ifAbsent: () => 1);
  }

  Future<bool> _removeFromDisk(
    String directory,
    String key,
    String channelId, {
    Set<String>? targets,
    bool deferIfUnavailable = false,
  }) async {
    try {
      final result = await _runDisk(
        _DiskRequest(
          path: _path(directory, key),
          action: _DiskAction.removeChannel,
          page: (channelId: channelId, rootId: ''),
          targets: targets,
        ),
        _acquireLease,
      );
      if (result != null) {
        if (result.reset) _reportReset();
        if (deferIfUnavailable || !result.reset) return true;
      }
    } catch (error, stackTrace) {
      _report(error, stackTrace);
    }
    if (deferIfUnavailable) return false;
    // If SQL cannot remove that channel's data, discard the disposable scope.
    await _sweep(directory, removedScope: key);
    return true;
  }

  /// Removes retired identity databases from interrupted account cleanup and
  /// incompatible cache formats left by an app update.
  Future<void> removeOrphans(Set<SavedMessageScope> retained) => _enqueue(
    (directory) => _sweep(directory, retained: retained.map(_scopeKey).toSet()),
  );

  Future<_DiskResult?> _forPage(
    SavedMessageTicket ticket,
    _DiskAction action, {
    List<NostrEvent> events = const [],
    String? relayAuthor,
  }) => _enqueue<_DiskResult?>((directory) async {
    if (!isCurrent(ticket)) return null;
    final result = await _runDisk(
      _DiskRequest(
        path: _path(directory, ticket._scope),
        action: action,
        page: ticket._page,
        events: events,
        relayAuthor: relayAuthor,
        payloadBudget: payloadBudget,
      ),
      _acquireLease,
    );
    if (result?.reset ?? false) _reportReset();
    return result;
  });

  Future<T?> _enqueue<T>(Future<T> Function(String directory) operation) {
    final next = _tail.then((_) async {
      try {
        final root = await _root();
        if (root == null) return null;
        return await operation(p.join(root.path, 'saved_messages'));
      } catch (error, stackTrace) {
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

final class _PendingEventRemoval {
  Set<String> targets = {};
  bool queued = false;
  bool deferred = false;

  void addTargets(Set<String> incoming) {
    for (final target in incoming) {
      if (targets.length > _maxWindowEvents) break;
      targets.add(target);
    }
  }
}

String _scopeKey(SavedMessageScope scope) => sha256
    .convert(utf8.encode('${scope.communityId}\n${scope.pubkey.toLowerCase()}'))
    .toString();
