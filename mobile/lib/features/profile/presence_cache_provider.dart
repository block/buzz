import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/agents/remote_agents.dart';
import '../../shared/agents/owned_agent_directory.dart';
import '../../shared/relay/relay.dart';

/// Community-scoped presence: absent means unknown. Polls recover expired leases.
///
/// Presence is a best-effort estimate between foreground polls. Per-key
/// revisions fence stale query completions. Snapshot/live ordering is not
/// causal: snapshots carry relay time and live events carry signer time. A
/// live event at/below the relay snapshot second is confirmed by a fresh
/// snapshot (persistence precedes fan-out). An empty snapshot has no relay
/// second, so a delayed older live event can briefly supersede its Offline;
/// the next 60-second poll corrects it.
class PresenceCacheNotifier extends Notifier<Map<String, String>> {
  /// Bound deduplication memory; redelivery must not re-arm confirmation.
  static const _seenIdLimit = 256;

  final Set<String> _tracked = {};
  final Set<String> _pending = {};
  final Map<String, int> _revisions = {};
  final Map<String, int> _timestamps = {};
  final Map<String, int> _snapshotAt = {};
  final List<String> _seenIds = [];
  // Keys the live subscription asks for. Only tracked keys are subscribed:
  // a community's presence heartbeats arrive many times a second.
  final Set<String> _subscribedKeys = {};
  void Function()? _presenceUnsub;
  Timer? _poll;
  Timer? _queued;
  Timer? _resubscribe;
  int _nextSubscription = 0;
  int _activeSubscription = 0;
  bool _subscribeAttempted = false;
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
    _snapshotAt.clear();
    _seenIds.clear();
    _subscribedKeys.clear();
    _activeSubscription = 0;
    _subscribeAttempted = false;
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
      _resubscribe?.cancel();
      _resubscribe = null;
    });
    if (sessionState.status == SessionStatus.connected &&
        lifecycle == AppLifecycleState.resumed) {
      _poll = Timer.periodic(const Duration(seconds: 60), (_) {
        if (_presenceUnsub == null || !_subscribedKeys.containsAll(_tracked)) {
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
    var added = false;
    for (final key in pubkeys.map((key) => key.trim().toLowerCase())) {
      if (key.isNotEmpty && _tracked.add(key)) {
        _pending.add(key);
        added = true;
      }
    }
    // While live (the poll runs only then), new keys are read once the
    // subscription covers them, so no live event falls between the snapshot
    // and the subscription.
    if (added && (_poll?.isActive ?? false)) {
      _queueResubscribe(_generation);
    } else {
      _schedule(_generation);
    }
  }

  void _queueResubscribe(int generation) {
    _resubscribe ??= Timer(Duration.zero, () {
      _resubscribe = null;
      // Without a subscription, the next poll opens one with every key.
      if (generation == _generation &&
          (_presenceUnsub != null || !_subscribeAttempted)) {
        unawaited(_subscribe(generation));
      }
    });
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
    if (_opening ||
        generation != _generation ||
        _tracked.isEmpty ||
        (_presenceUnsub != null && _subscribedKeys.containsAll(_tracked))) {
      return;
    }
    _opening = true;
    _subscribeAttempted = true;
    final keys = _tracked.toList()..sort();
    final subscription = ++_nextSubscription;
    var closed = false;
    // The subscription being replaced keeps delivering until this one opens.
    bool current() =>
        generation == _generation &&
        !closed &&
        subscription == _activeSubscription;
    try {
      final unsub = await ref
          .read(relaySessionProvider.notifier)
          .subscribeWithStatus(
            NostrFilter(
              kinds: const [EventKind.presenceUpdate],
              authors: keys,
              limit: 0,
            ),
            (event) {
              if (current() && !_retrying) _handlePresenceEvent(event);
            },
            onStatusChanged: (status) {
              if (!current()) return;
              _retrying = status == RelaySubscriptionStatus.retrying;
              _invalidateSnapshot();
              if (!_retrying) _schedule(generation);
            },
            onClosed: (_) {
              if (generation != _generation || closed) return;
              closed = true;
              if (subscription != _activeSubscription) return;
              _presenceUnsub?.call();
              _presenceUnsub = null;
              _subscribedKeys.clear();
              _invalidateSnapshot();
            },
          );
      if (generation != _generation || closed) {
        unsub();
        return;
      }
      final previous = _presenceUnsub;
      _activeSubscription = subscription;
      _retrying = false;
      _presenceUnsub = unsub;
      _subscribedKeys
        ..clear()
        ..addAll(keys);
      previous?.call();
      // Keys tracked while this one opened need a wider subscription.
      if (!_subscribedKeys.containsAll(_tracked)) _queueResubscribe(generation);
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
        event.createdAt < (_timestamps[pubkey] ?? 0) ||
        _seenRecently(event.id)) {
      return;
    }
    // Accepted evidence fences in-flight queries, even if it needs confirmation.
    _revisions[pubkey] = (_revisions[pubkey] ?? 0) + 1;
    // Equality cannot order events within the relay's observation second.
    if (event.createdAt <= (_snapshotAt[pubkey] ?? 0)) {
      _pending.add(pubkey);
      _schedule(_generation);
      return;
    }
    _timestamps[pubkey] = event.createdAt;
    if (state[pubkey] != status) state = {...state, pubkey: status};
  }

  Future<void> _refresh(int generation) async {
    if (_querying || _pending.isEmpty) return;
    _querying = true;
    final epoch = _epoch;
    final session = ref.read(relaySessionProvider.notifier);
    while (_pending.isNotEmpty) {
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
      var observedAt = 0;
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
        if (event.createdAt > observedAt) observedAt = event.createdAt;
      }
      final updated = {...state};
      for (final key in keys) {
        if ((_revisions[key] ?? 0) != revisions[key]) continue;
        if (events == null) {
          updated.remove(key); // failure is unknown; next poll retries
        } else {
          // Shared synthesis clock fences the batch; empty answers retain it.
          if (observedAt > (_snapshotAt[key] ?? 0)) {
            _snapshotAt[key] = observedAt;
          }
          updated[key] = latest[key]?.content.trim() ?? 'offline';
        }
      }
      if (!mapEquals(state, updated)) state = updated;
    }
    _querying = false;
  }

  bool _seenRecently(String id) {
    if (id.isEmpty) return false;
    if (_seenIds.contains(id)) return true;
    _seenIds.add(id);
    if (_seenIds.length > _seenIdLimit) _seenIds.removeAt(0);
    return false;
  }

  bool _validStatus(String status) =>
      status == 'online' || status == 'away' || status == 'offline';
}

final presenceCacheProvider =
    NotifierProvider<PresenceCacheNotifier, Map<String, String>>(
      PresenceCacheNotifier.new,
    );

/// Presentation shared by profiles, DMs and mentions. Remote agents wake on
/// messages. Their verified Builderlab runtime registration is enough to show
/// Online when the account API is unavailable or signed out, unless the owner
/// has a saved registry snapshot that excludes it. The registry also supports
/// older agents without runtime metadata. Local presence is unchanged.
final presentedPresenceProvider = Provider<Map<String, String>>((ref) {
  final presence = ref.watch(presenceCacheProvider);
  final directoryState = ref.watch(ownedAgentDirectoryProvider);
  final directory = directoryState.isLoading || directoryState.hasError
      ? null
      : directoryState.asData?.value;
  final controller = ref.watch(remoteAgentControllerProvider);
  void changed() => ref.invalidateSelf();
  controller.addListener(changed);
  ref.onDispose(() => controller.removeListener(changed));
  final saved = ref.watch(savedRemoteAvailabilityProvider);
  final remote = {
    ...?ref.watch(availableRemoteAgentsProvider).value,
    if (!controller.registryLoaded &&
        !saved.isLoading &&
        !saved.hasError &&
        saved.hasValue)
      for (final agent in directory ?? <OwnedAgentDirectoryEntry>[])
        if (agent.runtime == 'block.builderlab/builderlab' &&
            (saved.value == null || saved.value!.contains(agent.pubkey)))
          agent.pubkey,
  }..removeAll(controller.deletedAgents);
  if (remote.isEmpty) return presence;
  return {...presence, for (final pubkey in remote) pubkey: 'online'};
});
