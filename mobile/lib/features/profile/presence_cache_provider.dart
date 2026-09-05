import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';

/// Community-scoped presence: missing keys mean unknown, not offline.
/// Live updates overlap authenticated snapshots; foreground polling observes
/// relay lease expiry and retries failed reads without a tight retry loop.
class PresenceCacheNotifier extends Notifier<Map<String, String>> {
  final Set<String> _tracked = {};
  final Set<String> _pending = {};
  final Map<String, int> _revisions = {};
  final Map<String, int> _timestamps = {};
  void Function()? _presenceUnsub;
  Timer? _poll;
  Timer? _queued;
  int _generation = 0;
  int _epoch = 0;
  bool _querying = false;
  bool _opening = false;
  bool _retrying = false;
  (String, String?)? _scope;

  @override
  Map<String, String> build() {
    final sessionState = ref.watch(relaySessionProvider);
    final lifecycle = ref.watch(appLifecycleProvider);
    final scope = ref.watch(
      relayConfigProvider.select((config) => (config.baseUrl, config.nsec)),
    );
    if (_scope != scope) _tracked.clear();
    _scope = scope;
    final generation = ++_generation;
    _epoch++;
    _querying = _opening = _retrying = false;
    _revisions.clear();
    _timestamps.clear();
    _pending
      ..clear()
      ..addAll(_tracked);
    ref.onDispose(() {
      _generation++;
      _presenceUnsub?.call();
      _presenceUnsub = null;
      _poll?.cancel();
      _queued?.cancel();
      _queued = null;
    });
    if (sessionState.status == SessionStatus.connected &&
        lifecycle == AppLifecycleState.resumed) {
      _poll = Timer.periodic(const Duration(seconds: 60), (_) {
        if (_presenceUnsub == null) {
          unawaited(_subscribe(generation));
        } else if (!_querying) {
          _pending.addAll(_tracked);
          _schedule(generation);
        }
      });
      // A transport may report ready synchronously; initialize state first.
      Future.microtask(() => _subscribe(generation));
    }
    return {};
  }

  /// Request initial presence for exact keys. Repeated render calls coalesce.
  void track(List<String> pubkeys) {
    for (final key in pubkeys.map((key) => key.trim().toLowerCase())) {
      if (key.isNotEmpty && _tracked.add(key)) _pending.add(key);
    }
    _schedule(_generation);
  }

  void _schedule(int generation) {
    _queued ??= Timer(Duration.zero, () {
      _queued = null;
      if (generation == _generation && _presenceUnsub != null && !_retrying) {
        unawaited(_refresh(generation));
      }
    });
  }

  void _invalidateSnapshot() {
    _epoch++;
    _querying = false;
    _pending.addAll(_tracked);
    if (state.isNotEmpty) state = {};
  }

  Future<void> _subscribe(int generation) async {
    if (_opening || generation != _generation) return;
    _opening = true;
    _retrying = false;
    var closed = false;
    try {
      final unsub = await ref
          .read(relaySessionProvider.notifier)
          .subscribeWithStatus(
            const NostrFilter(kinds: [EventKind.presenceUpdate], limit: 0),
            (event) {
              if (generation == _generation && !closed && !_retrying) {
                _handlePresenceEvent(event);
              }
            },
            onStatusChanged: (status) {
              if (generation != _generation || closed) return;
              _retrying = status == RelaySubscriptionStatus.retrying;
              _invalidateSnapshot();
              if (!_retrying) _schedule(generation);
            },
            onClosed: (_) {
              if (generation != _generation || closed) return;
              closed = true;
              _presenceUnsub?.call();
              _presenceUnsub = null;
              _invalidateSnapshot();
            },
          );
      if (generation != _generation || closed) {
        unsub();
        return;
      }
      _presenceUnsub = unsub;
      _schedule(generation);
    } catch (error) {
      if (generation == _generation) {
        _invalidateSnapshot();
        debugPrint('[PresenceCacheNotifier] subscription failed: $error');
      }
    } finally {
      if (generation == _generation) _opening = false;
    }
  }

  void _handlePresenceEvent(NostrEvent event) {
    final pubkey = event.pubkey.toLowerCase();
    final status = event.content.trim();
    if (event.kind != EventKind.presenceUpdate ||
        !_tracked.contains(pubkey) ||
        !_validStatus(status) ||
        event.createdAt < (_timestamps[pubkey] ?? 0)) {
      return;
    }
    _timestamps[pubkey] = event.createdAt;
    // Even an unchanged heartbeat must fence an older in-flight snapshot.
    _revisions[pubkey] = (_revisions[pubkey] ?? 0) + 1;
    if (state[pubkey] != status) state = {...state, pubkey: status};
  }

  Future<void> _refresh(int generation) async {
    if (_querying || _pending.isEmpty) return;
    _querying = true;
    final epoch = _epoch;
    final session = ref.read(relaySessionProvider.notifier);
    while (_pending.isNotEmpty) {
      // The bridge synthesizes one current record per author. Explicitly cap
      // batches and cover every requested key instead of relying on limit=100.
      final keys = _pending.take(100).toList();
      _pending.removeAll(keys);
      final revisions = {for (final key in keys) key: _revisions[key] ?? 0};
      List<NostrEvent>? events;
      try {
        events = await session.queryRelay([
          NostrFilter(
            kinds: [EventKind.presenceUpdate],
            authors: keys,
            limit: keys.length,
          ),
        ]);
      } catch (error) {
        debugPrint('[PresenceCacheNotifier] snapshot failed: $error');
      }
      if (generation != _generation || epoch != _epoch) return;
      final latest = <String, NostrEvent>{};
      for (final event in events ?? <NostrEvent>[]) {
        final key = (event.getTagValue('p') ?? event.pubkey).toLowerCase();
        if (!revisions.containsKey(key) ||
            event.kind != EventKind.presenceUpdate ||
            !_validStatus(event.content.trim())) {
          continue;
        }
        if (!latest.containsKey(key) ||
            event.createdAt > latest[key]!.createdAt) {
          latest[key] = event;
        }
      }
      final updated = {...state};
      for (final key in keys) {
        if ((_revisions[key] ?? 0) != revisions[key]) continue;
        if (events == null) {
          updated.remove(key); // failure is unknown; next poll retries
        } else {
          updated[key] = latest[key]?.content.trim() ?? 'offline';
        }
      }
      if (!mapEquals(state, updated)) state = updated;
    }
    _querying = false;
  }

  bool _validStatus(String status) =>
      status == 'online' || status == 'away' || status == 'offline';
}

final presenceCacheProvider =
    NotifierProvider<PresenceCacheNotifier, Map<String, String>>(
      PresenceCacheNotifier.new,
    );
