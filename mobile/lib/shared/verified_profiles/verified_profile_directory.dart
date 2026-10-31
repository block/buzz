import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../relay/relay.dart';
import 'verified_profile.dart';
import 'verified_profile_discovery_fetch.dart';
import 'verified_profile_proof.dart';

const _wantedLimit = 1024;
const _eoseTimeout = Duration(seconds: 10);
const _discoveryRefresh = Duration(minutes: 5);

/// A relay returns one current projection per subject; a few duplicates are
/// tolerated, a flood before EOSE fails the batch instead of buffering it.
const _maxEarlyPerSubject = 4;

class _Batch {
  _Batch(this.subjects);
  final List<String> subjects;
  final List<NostrEvent> early = [];

  /// Live events awaiting proof, newest per subject, so a fast stream cannot
  /// queue unbounded work.
  final Map<String, NostrEvent> pending = {};
  Future<void> tail = Future.value();

  /// Bumped when the relay replays the subscription, so work from the
  /// previous snapshot is dropped.
  int epoch = 0;
  void Function()? unsubscribe;

  /// Retires this batch so the next sync can reopen it.
  void Function() fail = () {};
  Timer? timeout;
  bool active = true;
  bool ready = false;
  int candidates = 0;

  void stop() {
    active = false;
    timeout?.cancel();
    unsubscribe?.call();
    unsubscribe = null;
  }
}

/// Verified profiles for the community's currently relevant subjects, keyed by
/// lowercase pubkey. Empty unless a successful discovery enables the feature.
///
/// Records survive disconnects as `checked: false` (cached) and are only
/// reconciled by a completed subscription on the current connection.
class VerifiedProfilesNotifier extends Notifier<Map<String, VerifiedProfile>> {
  final Map<String, int> _wanted = {};
  final Map<String, NostrEvent> _heads = {};
  final Set<String> _checked = {};
  final Map<String, _Batch> _batches = {};
  VerifiedProfileAuthority? _authority;
  bool _enabled = false;
  bool _discoveryFresh = false;
  bool _connected = false;
  bool _discoveryInFlight = false;
  bool _scheduled = false;
  int _generation = 0;
  int _interest = 0;
  (String, String?)? _scope;
  Timer? _refreshTimer;

  @override
  Map<String, VerifiedProfile> build() {
    final scope = ref.watch(
      relayConfigProvider.select((config) => (config.baseUrl, config.nsec)),
    );
    final status = ref.watch(
      relaySessionProvider.select((session) => session.status),
    );
    if (_scope != null && _scope != scope) _resetCommunity();
    _scope = scope;
    final generation = ++_generation;
    _connected = status == SessionStatus.connected;
    _stopBatches();
    _discoveryFresh = false;
    _discoveryInFlight = false;
    ref.onDispose(() {
      _generation++;
      _stopBatches();
      _refreshTimer?.cancel();
      _refreshTimer = null;
    });
    if (_connected && _wanted.isNotEmpty) {
      Future.microtask(() => _onConnected(generation));
    }
    return _snapshot();
  }

  void _resetCommunity() {
    _wanted.clear();
    _heads.clear();
    _authority = null;
    _enabled = false;
  }

  void _stopBatches() {
    for (final batch in _batches.values) {
      batch.stop();
    }
    _batches.clear();
    _checked.clear();
  }

  /// Marks [pubkeys] relevant. Repeated calls coalesce into one scheduling pass.
  void track(Iterable<String> pubkeys) {
    final valid = RegExp(r'^[0-9a-f]{64}$');
    var added = false;
    for (final raw in pubkeys) {
      final key = raw.trim().toLowerCase();
      if (!valid.hasMatch(key)) continue;
      added |= !_wanted.containsKey(key);
      _wanted[key] ??= ++_interest;
    }
    while (_wanted.length > _wantedLimit) {
      final oldest = _wanted.entries.reduce(
        (a, b) => a.value < b.value ? a : b,
      );
      _wanted.remove(oldest.key);
      _heads.remove(oldest.key);
      _checked.remove(oldest.key);
      added = true;
    }
    if (!added) return;
    if (_authority == null && !_discoveryInFlight && _connected) {
      unawaited(_refreshDiscovery(_generation));
    } else {
      _schedule(_generation);
    }
  }

  Future<void> _onConnected(int generation) async {
    if (generation != _generation) return;
    await _refreshDiscovery(generation);
  }

  void _armRefresh(int generation) {
    _refreshTimer?.cancel();
    _refreshTimer = null;
    if (generation != _generation || !_connected || _wanted.isEmpty) return;
    _refreshTimer = Timer(_discoveryRefresh, () {
      unawaited(_refreshDiscovery(generation));
    });
  }

  Future<void> _refreshDiscovery(int generation) async {
    if (_discoveryInFlight || generation != _generation || !_connected) return;
    _discoveryInFlight = true;
    try {
      final result = await ref.read(verifiedProfileDiscoveryFetchProvider)();
      if (generation != _generation) return;
      _discoveryFresh = true;
      switch (result) {
        case VerifiedProfilesDisabled():
          // Hide marks but keep the historical records.
          _enabled = false;
          _stopBatches();
        case VerifiedProfilesEnabled(:final authority):
          if (_authority != null && _authority!.pubkey != authority.pubkey) {
            _stopBatches();
            _heads.clear();
          }
          _authority = authority;
          _enabled = true;
          _schedule(generation);
      }
    } catch (error) {
      // Failed or malformed discovery is not a disable: keep the last state,
      // but it is no longer fresh.
      if (generation == _generation) _discoveryFresh = false;
      debugPrint('[VerifiedProfiles] discovery failed: $error');
    } finally {
      if (generation == _generation) {
        _discoveryInFlight = false;
        _armRefresh(generation);
        _publish();
      }
    }
  }

  void _schedule(int generation) {
    if (_scheduled) return;
    _scheduled = true;
    scheduleMicrotask(() {
      _scheduled = false;
      if (generation == _generation) _syncBatches(generation);
    });
  }

  void _syncBatches(int generation) {
    final authority = _authority;
    if (!_connected || !_enabled || authority == null) {
      _publish();
      return;
    }
    final subjects = _wanted.keys.toList();
    final next = <String, List<String>>{};
    for (var i = 0; i < subjects.length; i += verifiedProfileBatchSize) {
      final end = i + verifiedProfileBatchSize;
      final chunk = subjects.sublist(
        i,
        end < subjects.length ? end : subjects.length,
      );
      next[chunk.join(':')] = chunk;
    }
    for (final id in _batches.keys.toList()) {
      if (next.containsKey(id)) continue;
      final batch = _batches.remove(id)!..stop();
      _checked.removeAll(batch.subjects);
    }
    for (final MapEntry(key: id, value: chunk) in next.entries) {
      if (!_batches.containsKey(id)) {
        unawaited(_open(id, chunk, authority, generation));
      }
    }
    if (_refreshTimer == null) _armRefresh(generation);
    _publish();
  }

  Future<void> _open(
    String id,
    List<String> chunk,
    VerifiedProfileAuthority authority,
    int generation,
  ) async {
    final batch = _Batch(chunk);
    _batches[id] = batch;
    void fail() {
      if (!batch.active) return;
      batch.stop();
      if (_batches[id] == batch) _batches.remove(id);
      _checked.removeAll(chunk);
      _publish();
      // No hot retry: the next discovery refresh, reconnect or tracked
      // subject reopens it.
    }

    batch.fail = fail;

    batch.timeout = Timer(_eoseTimeout, fail);
    try {
      final unsubscribe = await ref
          .read(relaySessionProvider.notifier)
          .subscribeWithStatus(
            NostrFilter(
              kinds: const [verifiedProfileKind],
              authors: [authority.pubkey],
              tags: {'#d': chunk},
              limit: chunk.length,
            ),
            (event) => _onEvent(batch, event, authority, generation),
            onStatusChanged: (status) => switch (status) {
              RelaySubscriptionStatus.ready => _onEose(
                batch,
                authority,
                generation,
              ),
              RelaySubscriptionStatus.retrying => _onRetry(batch, fail),
            },
            onClosed: (_) => fail(),
          );
      if (!batch.active || generation != _generation) {
        unsubscribe();
      } else {
        batch.unsubscribe = unsubscribe;
      }
    } catch (error) {
      debugPrint('[VerifiedProfiles] subscribe failed: $error');
      fail();
    }
  }

  void _onEvent(
    _Batch batch,
    NostrEvent event,
    VerifiedProfileAuthority authority,
    int generation,
  ) {
    if (!batch.active || generation != _generation) return;
    final subject = verifiedProfileSubject(event, authority);
    if (subject == null || !batch.subjects.contains(subject)) return;
    if (!batch.ready) {
      if (batch.early.length >= batch.subjects.length * _maxEarlyPerSubject) {
        debugPrint('[VerifiedProfiles] too many events before EOSE');
        batch.fail();
        return;
      }
      batch.candidates++;
      batch.early.add(event);
      return;
    }
    final queued = batch.pending.isNotEmpty;
    if (supersedes(event, batch.pending[subject])) {
      batch.pending[subject] = event;
    }
    if (queued) return;
    final epoch = batch.epoch;
    _enqueue(batch, generation, () async {
      final events = batch.pending.values.toList();
      batch.pending.clear();
      final proven = await _prove(events);
      if (_current(batch, generation, epoch)) {
        _accept(proven, authority);
        _publish();
      }
    });
  }

  /// The relay closed and is replaying the subscription: reconcile the replay
  /// as a new snapshot rather than as live updates.
  void _onRetry(_Batch batch, void Function() fail) {
    if (!batch.active) return;
    batch.epoch++;
    batch.ready = false;
    batch.candidates = 0;
    batch.early.clear();
    batch.pending.clear();
    batch.timeout?.cancel();
    batch.timeout = Timer(_eoseTimeout, fail);
    _checked.removeAll(batch.subjects);
    _publish();
  }

  void _onEose(
    _Batch batch,
    VerifiedProfileAuthority authority,
    int generation,
  ) {
    if (!batch.active || batch.ready || generation != _generation) return;
    batch.ready = true;
    batch.timeout?.cancel();
    batch.timeout = null;
    final early = List.of(batch.early);
    batch.early.clear();
    final epoch = batch.epoch;
    final candidates = batch.candidates;
    _enqueue(batch, generation, () async {
      final proven = await _prove(early);
      if (!_current(batch, generation, epoch)) return;
      _accept(proven, authority);
      final seen = {
        for (final event in proven) verifiedProfileSubject(event, authority)!,
      };
      // A full response that omits subjects may be truncated: only a short
      // response proves absence.
      final complete =
          candidates < batch.subjects.length ||
          seen.length == batch.subjects.length;
      for (final subject in batch.subjects) {
        if (!complete && !seen.contains(subject)) continue;
        _checked.add(subject);
        if (!seen.contains(subject)) _heads.remove(subject);
      }
      _publish();
    });
  }

  bool _live(_Batch batch, int generation) =>
      batch.active && generation == _generation && ref.mounted;

  bool _current(_Batch batch, int generation, int epoch) =>
      _live(batch, generation) && batch.epoch == epoch;

  void _enqueue(_Batch batch, int generation, Future<void> Function() job) {
    final epoch = batch.epoch;
    batch.tail = batch.tail.then((_) async {
      if (!_live(batch, generation) || batch.epoch != epoch) return;
      try {
        await job();
      } catch (error) {
        // Retire the batch so a later sync reconciles it again.
        debugPrint('[VerifiedProfiles] batch job failed: $error');
        batch.fail();
      }
    });
  }

  Future<List<NostrEvent>> _prove(List<NostrEvent> events) async {
    if (events.isEmpty) return const [];
    final results = await ref.read(verifiedProfileProofCheckerProvider)(events);
    return [
      for (var i = 0; i < events.length; i++)
        if (results[i]) events[i],
    ];
  }

  void _accept(List<NostrEvent> events, VerifiedProfileAuthority authority) {
    for (final event in events) {
      final subject = verifiedProfileSubject(event, authority);
      if (subject == null || !_wanted.containsKey(subject)) continue;
      if (supersedes(event, _heads[subject])) _heads[subject] = event;
    }
  }

  Map<String, VerifiedProfile> _snapshot() {
    final authority = _authority;
    if (!_enabled || authority == null) return const {};
    return {
      for (final MapEntry(:key, :value) in _heads.entries)
        if (_wanted.containsKey(key))
          key: ?parseVerifiedProfile(
            value,
            authority,
            checked: _discoveryFresh && _checked.contains(key),
          ),
    };
  }

  void _publish() {
    if (!ref.mounted) return;
    final next = _snapshot();
    if (!mapEquals(state, next)) state = next;
  }
}

final verifiedProfilesProvider =
    NotifierProvider<VerifiedProfilesNotifier, Map<String, VerifiedProfile>>(
      VerifiedProfilesNotifier.new,
    );
