import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../crypto/nip44.dart';
import '../relay/relay.dart';
import 'agent_usage_models.dart';

/// Bound on tracked recent event ids so a long-lived session subscription
/// cannot grow this set without limit (mirrors the observer subscription's
/// `_maxObserverEvents` cap).
const _maxRecentEventIds = 500;
const _maxTrackedAgents = 64;
const _maxTrackedScopes = 256;
const _historyPageSize = 200;
const _maxHistoryPages = 25;

typedef AgentUsageScopeKey = ({
  String agentPubkey,
  String channelId,
  String? threadRootId,
});
typedef AgentUsageHistoryKey = ({
  String agentPubkey,
  String? channelId,
  String? threadRootId,
});

bool _hasValidNip01Signature(NostrEvent event) {
  try {
    nostr.Event.fromMap(event.toJson(), verify: true);
    return true;
  } on Object {
    return false;
  }
}

/// Owner-scoped live state for all agents' usage snapshots.
@immutable
class AgentUsageRelayState {
  final AgentUsageConnectionState connection;
  final Map<String, AgentUsageSnapshot> snapshotsByAgent;
  final Map<AgentUsageScopeKey, AgentUsageSnapshot> scopedSnapshots;
  final String? errorMessage;

  const AgentUsageRelayState({
    required this.connection,
    required this.snapshotsByAgent,
    this.scopedSnapshots = const {},
    this.errorMessage,
  });

  const AgentUsageRelayState.initial()
    : connection = AgentUsageConnectionState.idle,
      snapshotsByAgent = const {},
      scopedSnapshots = const {},
      errorMessage = null;

  /// Returns agent-wide provider windows combined with context from the exact
  /// requested channel/thread. With no channel, returns the agent-wide view.
  AgentUsageSnapshot? snapshotFor({
    required String agentPubkey,
    String? channelId,
    String? threadRootId,
  }) {
    final normalizedAgent = agentPubkey.toLowerCase();
    final global = snapshotsByAgent[normalizedAgent];
    if (channelId == null) return global;

    final scoped =
        scopedSnapshots[(
          agentPubkey: normalizedAgent,
          channelId: channelId,
          threadRootId: threadRootId,
        )];
    if (global == null && scoped == null) return null;
    final descriptive = global ?? scoped!;
    final lastEventAt = global == null
        ? scoped!.lastEventAt
        : scoped == null || global.lastEventAt.isAfter(scoped.lastEventAt)
        ? global.lastEventAt
        : scoped.lastEventAt;
    return AgentUsageSnapshot(
      harness: descriptive.harness,
      model: descriptive.model,
      lastEventAt: lastEventAt,
      contextUsedTokens: scoped?.contextUsedTokens,
      contextLimitTokens: scoped?.contextLimitTokens,
      contextSnapshotAt: scoped?.contextSnapshotAt,
      accountUsageWindows: global?.accountUsageWindows ?? const [],
      accountUsageWindowsAt: global?.accountUsageWindowsAt,
      cumulative: scoped?.cumulative,
      cumulativeAt: scoped?.cumulativeAt,
    );
  }
}

/// Connection state for the usage relay subscription. Reuses the same shape
/// as the NIP-AO observer subscription's connection states.
enum AgentUsageConnectionState { idle, connecting, open, error }

/// Subscribes to `{kinds:[44200], #p:[ownerPubkey]}` (NIP-AM Client
/// Behavior), decrypts each event with NIP-44, and folds decrypted payloads
/// into a per-agent [AgentUsageSnapshot] via [mergeAgentUsageSnapshot].
///
/// Per-event decrypt/parse failures are dropped silently and do NOT surface
/// as a connection error — NIP-AM's Client Behavior section requires clients
/// to "ignore events that fail to decrypt or parse", since one malformed or
/// foreign-keyed event must not degrade the usage view for every other
/// agent. Only subscription-level failures (bad local key, relay CLOSED)
/// set [AgentUsageRelayState.errorMessage].
class AgentUsageRelayNotifier extends Notifier<AgentUsageRelayState> {
  final Map<String, AgentUsageSnapshot> _snapshotsByAgent = {};
  final Map<AgentUsageScopeKey, AgentUsageSnapshot> _scopedSnapshots = {};
  final List<String> _agentLru = [];
  final List<AgentUsageScopeKey> _scopeLru = [];
  final Set<String> _trackedAgents = {};
  final Set<AgentUsageHistoryKey> _historyRequests = {};
  final Set<AgentUsageHistoryKey> _historyInFlight = {};
  final Set<AgentUsageHistoryKey> _historyCompleted = {};
  final List<String> _recentEventIds = [];
  final Set<String> _recentEventIdSet = {};
  final Map<String, Uint8List> _conversationKeysByAgent = {};

  void Function()? _unsubscribe;
  Future<void>? _startFuture;
  String? _privHex;
  String? _ownerPubkey;
  String? _identityKey;
  String? _errorMessage;
  int _subscriptionEpoch = 0;
  bool _disposed = false;

  @override
  AgentUsageRelayState build() {
    final config = ref.watch(relayConfigProvider);
    final sessionState = ref.watch(relaySessionProvider);
    final nsec = config.nsec;
    String publicIdentity = '';
    if (nsec?.isNotEmpty == true) {
      try {
        publicIdentity = _derivePubkey(_decodePrivkey(nsec!));
      } on Object {
        publicIdentity = 'invalid';
      }
    }
    final identityKey = '${config.baseUrl}|$publicIdentity';

    _disposed = false;
    if (_identityKey != null && _identityKey != identityKey) {
      _reset();
    }
    _identityKey = identityKey;

    ref.onDispose(() {
      _disposed = true;
      _reset();
    });

    if (sessionState.status == SessionStatus.connected) {
      Future.microtask(_ensureSubscribed);
    }

    final hasSigningKey = config.nsec?.isNotEmpty == true;
    return AgentUsageRelayState(
      connection: hasSigningKey
          ? _connectionForSession(sessionState.status)
          : AgentUsageConnectionState.idle,
      snapshotsByAgent: _snapshotFrames(),
      scopedSnapshots: _scopedSnapshotFrames(),
      errorMessage: _errorMessage,
    );
  }

  /// Registers an agent already identified by trusted local channel/profile
  /// state. Unknown relay authors are never decrypted or retained.
  void trackAgent(String agentPubkey) {
    final normalizedAgent = agentPubkey.toLowerCase();
    if (!RegExp(r'^[0-9a-f]{64}$').hasMatch(normalizedAgent)) return;
    if (!_trackedAgents.contains(normalizedAgent) &&
        _trackedAgents.length >= _maxTrackedAgents) {
      _evictAgent(_trackedAgents.first);
    }
    _trackedAgents
      ..remove(normalizedAgent)
      ..add(normalizedAgent);
  }

  /// Registers a known agent and starts a bounded owner-scoped backfill.
  Future<void> ensureAgentHistory({
    required String agentPubkey,
    String? channelId,
    String? threadRootId,
  }) {
    final normalizedAgent = agentPubkey.toLowerCase();
    trackAgent(normalizedAgent);
    if (!_trackedAgents.contains(normalizedAgent)) return Future.value();
    final request = (
      agentPubkey: normalizedAgent,
      channelId: channelId,
      threadRootId: threadRootId,
    );
    if (!_rememberHistoryRequest(request)) return Future.value();
    return _startHistoryBackfill(request);
  }

  bool _rememberHistoryRequest(AgentUsageHistoryKey request) {
    if (_historyRequests.remove(request)) {
      _historyRequests.add(request);
      return true;
    }
    if (_historyRequests.length >= _maxTrackedScopes) {
      AgentUsageHistoryKey? evicted;
      for (final candidate in _historyRequests) {
        if (!_historyInFlight.contains(candidate)) {
          evicted = candidate;
          break;
        }
      }
      if (evicted == null) return false;
      _historyRequests.remove(evicted);
      _historyCompleted.remove(evicted);
    }
    _historyRequests.add(request);
    return true;
  }

  Future<void> _startHistoryBackfill(AgentUsageHistoryKey request) async {
    final ownerPubkey = _ownerPubkey;
    final epoch = _subscriptionEpoch;
    if (ownerPubkey == null ||
        _historyCompleted.contains(request) ||
        !_historyInFlight.add(request)) {
      return;
    }

    try {
      final session = ref.read(relaySessionProvider.notifier);
      int? until;
      var completed = false;
      final seenEventIds = <String>{};
      for (var page = 0; page < _maxHistoryPages; page++) {
        final events = await session.fetchHistory(
          NostrFilter(
            kinds: [EventKind.agentTurnMetric],
            authors: [request.agentPubkey],
            tags: {
              '#p': [ownerPubkey],
            },
            limit: _historyPageSize,
            until: until,
          ),
        );
        if (_disposed || epoch != _subscriptionEpoch) return;
        var newEventCount = 0;
        for (final event in events) {
          if (seenEventIds.add(event.id)) {
            newEventCount += 1;
            _handleEvent(event, emit: false);
          }
        }
        if (events.isNotEmpty) {
          _emit(connection: AgentUsageConnectionState.open);
        }
        if (_hasRequiredHistory(request)) {
          completed = true;
          break;
        }
        if (events.length < _historyPageSize) {
          completed = true;
          break;
        }
        final oldest = events
            .map((event) => event.createdAt)
            .reduce((a, b) => a < b ? a : b);
        final nextUntil = oldest;
        if (nextUntil < 0 || (until != null && nextUntil > until)) break;
        if (until == nextUntil && newEventCount == 0) break;
        until = nextUntil;
      }
      if (!_disposed &&
          epoch == _subscriptionEpoch &&
          completed &&
          _historyRequests.contains(request)) {
        _historyCompleted.add(request);
      }
    } catch (error) {
      debugPrint('[AgentUsage] historical reconstruction failed: $error');
    } finally {
      if (epoch == _subscriptionEpoch) {
        _historyInFlight.remove(request);
      }
    }
  }

  bool _hasRequiredHistory(AgentUsageHistoryKey request) {
    final global = _snapshotsByAgent[request.agentPubkey];
    final scoped = request.channelId == null
        ? global
        : _scopedSnapshots[(
            agentPubkey: request.agentPubkey,
            channelId: request.channelId!,
            threadRootId: request.threadRootId,
          )];
    final hasContext =
        scoped?.contextUsedTokens != null && scoped?.contextLimitTokens != null;
    final hasWindows = global?.accountUsageWindows.isNotEmpty == true;
    return hasContext && hasWindows;
  }

  Future<void> _ensureSubscribed() {
    if (_disposed) return Future.value();
    if (_unsubscribe != null) return Future.value();
    if (_startFuture != null) return _startFuture!;

    _startFuture = _subscribe(_subscriptionEpoch);
    return _startFuture!;
  }

  Future<void> _subscribe(int epoch) async {
    try {
      if (_disposed || epoch != _subscriptionEpoch) {
        return;
      }

      final config = ref.read(relayConfigProvider);
      final nsec = config.nsec;
      if (nsec == null || nsec.isEmpty) {
        _errorMessage = null;
        _emit(connection: AgentUsageConnectionState.idle);
        return;
      }

      final privHex = _decodePrivkey(nsec);
      final ownerPubkey = _derivePubkey(privHex);
      if (_disposed || epoch != _subscriptionEpoch) {
        return;
      }
      _privHex = privHex;
      _ownerPubkey = ownerPubkey;
      _errorMessage = null;
      _emit(connection: AgentUsageConnectionState.connecting);

      final session = ref.read(relaySessionProvider.notifier);
      final unsubscribe = await session.subscribe(
        NostrFilter(
          kinds: [EventKind.agentTurnMetric],
          tags: {
            '#p': [ownerPubkey],
          },
          limit: 200,
        ),
        _handleEvent,
        onClosed: (message) {
          _unsubscribe = null;
          _errorMessage = 'Agent usage subscription closed: $message';
          _emit(connection: AgentUsageConnectionState.error);
        },
      );

      if (_disposed || epoch != _subscriptionEpoch) {
        unsubscribe();
        return;
      }

      _unsubscribe = unsubscribe;
      _emit(connection: AgentUsageConnectionState.open);
      for (final request in _historyRequests.toList(growable: false)) {
        unawaited(_startHistoryBackfill(request));
      }
    } catch (error) {
      if (_disposed || epoch != _subscriptionEpoch) {
        return;
      }
      _errorMessage = _usageErrorMessage(error);
      _emit(connection: AgentUsageConnectionState.error);
    } finally {
      if (epoch == _subscriptionEpoch) {
        _startFuture = null;
      }
    }
  }

  void _handleEvent(NostrEvent event, {bool emit = true}) {
    if (event.kind != EventKind.agentTurnMetric ||
        event.content.length < nip44MinContentLength ||
        event.content.length > nip44MaxContentLength ||
        event.tags.any((tag) => tag.isNotEmpty && tag.first == 'h')) {
      return;
    }

    // Relays are untrusted. Verify the canonical event id and Schnorr
    // signature before deduplication so a forged duplicate cannot poison the
    // recent-id cache and suppress a later valid event.
    if (!_hasValidNip01Signature(event)) {
      return;
    }

    // NIP-AM: events MUST carry exactly one `p` tag (owner) and one `agent`
    // tag equal to `pubkey`. Tag matching alone is not an authorization
    // decision here (the relay already gates reads to the owner), but a
    // malformed envelope cannot be decrypted meaningfully either way.
    final agentTags = event.tags
        .where((tag) => tag.isNotEmpty && tag.first == 'agent')
        .toList(growable: false);
    final ownerTags = event.tags
        .where((tag) => tag.isNotEmpty && tag.first == 'p')
        .toList(growable: false);
    if (agentTags.length != 1 ||
        agentTags.single.length != 2 ||
        ownerTags.length != 1 ||
        ownerTags.single.length != 2) {
      return;
    }
    final agentPubkey = agentTags.single[1];
    final pTag = ownerTags.single[1];
    final canonicalPubkey = RegExp(r'^[0-9a-f]{64}$');
    if (!canonicalPubkey.hasMatch(event.pubkey) ||
        !canonicalPubkey.hasMatch(agentPubkey) ||
        !canonicalPubkey.hasMatch(pTag) ||
        event.pubkey != agentPubkey) {
      return;
    }

    final ownerPubkey = _ownerPubkey;
    final privHex = _privHex;
    if (ownerPubkey == null || privHex == null) {
      return;
    }

    if (pTag != ownerPubkey.toLowerCase()) {
      return;
    }

    final normalizedAgent = agentPubkey.toLowerCase();
    if (!_trackedAgents.contains(normalizedAgent)) return;
    if (!_recentEventIdSet.add(event.id)) {
      return;
    }
    _recentEventIds.add(event.id);
    if (_recentEventIds.length > _maxRecentEventIds) {
      final removed = _recentEventIds.removeAt(0);
      _recentEventIdSet.remove(removed);
    }
    final payload = _decryptPayload(event, normalizedAgent, privHex);
    if (payload == null) return;
    _touchAgent(normalizedAgent);

    final merged = mergeAgentUsageSnapshot(
      _snapshotsByAgent[normalizedAgent],
      payload,
      eventId: event.id,
    );
    _snapshotsByAgent[normalizedAgent] = merged;
    final channelId = payload.channelId;
    if (channelId != null) {
      final scopeKey = (
        agentPubkey: normalizedAgent,
        channelId: channelId,
        threadRootId: payload.threadRootId,
      );
      _touchScope(scopeKey);
      _scopedSnapshots[scopeKey] = mergeAgentUsageSnapshot(
        _scopedSnapshots[scopeKey],
        payload,
        eventId: event.id,
      );
    }
    if (emit) _emit(connection: AgentUsageConnectionState.open);
  }

  /// Decrypts and parses one event's payload. Returns null and drops the
  /// event silently on any failure, per NIP-AM Client Behavior — this MUST
  /// NOT be surfaced as a subscription-level error (see class doc).
  AgentTurnMetricPayload? _decryptPayload(
    NostrEvent event,
    String normalizedAgent,
    String privHex,
  ) {
    try {
      final conversationKey =
          _conversationKeysByAgent[normalizedAgent] ??
          getConversationKey(privHex, normalizedAgent);
      final plaintext = nip44Decrypt(conversationKey, event.content);
      final json = jsonDecode(plaintext) as Map<String, dynamic>;
      final payload = AgentTurnMetricPayload.fromJson(json);
      _conversationKeysByAgent[normalizedAgent] = conversationKey;
      return payload;
    } catch (error) {
      debugPrint('[AgentUsage] dropping unparseable kind:44200 event: $error');
      return null;
    }
  }

  void _emit({required AgentUsageConnectionState connection}) {
    if (_disposed) return;
    state = AgentUsageRelayState(
      connection: connection,
      snapshotsByAgent: _snapshotFrames(),
      scopedSnapshots: _scopedSnapshotFrames(),
      errorMessage: _errorMessage,
    );
  }

  Map<String, AgentUsageSnapshot> _snapshotFrames() {
    return Map<String, AgentUsageSnapshot>.unmodifiable(_snapshotsByAgent);
  }

  Map<AgentUsageScopeKey, AgentUsageSnapshot> _scopedSnapshotFrames() {
    return Map<AgentUsageScopeKey, AgentUsageSnapshot>.unmodifiable(
      _scopedSnapshots,
    );
  }

  void _touchAgent(String agentPubkey) {
    _agentLru.remove(agentPubkey);
    if (!_snapshotsByAgent.containsKey(agentPubkey) &&
        _snapshotsByAgent.length >= _maxTrackedAgents) {
      _evictAgent(_agentLru.first);
    }
    _agentLru.add(agentPubkey);
  }

  void _evictAgent(String agentPubkey) {
    _trackedAgents.remove(agentPubkey);
    _agentLru.remove(agentPubkey);
    _snapshotsByAgent.remove(agentPubkey);
    _conversationKeysByAgent.remove(agentPubkey);
    _historyRequests.removeWhere(
      (request) => request.agentPubkey == agentPubkey,
    );
    _historyCompleted.removeWhere(
      (request) => request.agentPubkey == agentPubkey,
    );
    _historyInFlight.removeWhere(
      (request) => request.agentPubkey == agentPubkey,
    );
    final evictedScopes = _scopeLru
        .where((scope) => scope.agentPubkey == agentPubkey)
        .toList(growable: false);
    for (final scope in evictedScopes) {
      _scopeLru.remove(scope);
      _scopedSnapshots.remove(scope);
    }
  }

  void _touchScope(AgentUsageScopeKey scope) {
    _scopeLru.remove(scope);
    if (!_scopedSnapshots.containsKey(scope) &&
        _scopedSnapshots.length >= _maxTrackedScopes) {
      _scopedSnapshots.remove(_scopeLru.removeAt(0));
    }
    _scopeLru.add(scope);
  }

  void _reset() {
    _subscriptionEpoch += 1;
    _unsubscribe?.call();
    _unsubscribe = null;
    _startFuture = null;
    _privHex = null;
    _ownerPubkey = null;
    _errorMessage = null;
    _snapshotsByAgent.clear();
    _scopedSnapshots.clear();
    _agentLru.clear();
    _scopeLru.clear();
    _trackedAgents.clear();
    _historyRequests.clear();
    _historyInFlight.clear();
    _historyCompleted.clear();
    _recentEventIds.clear();
    _recentEventIdSet.clear();
    _conversationKeysByAgent.clear();
  }

  static String _decodePrivkey(String nsec) {
    try {
      final privHex = nostr.Nip19.decode(payload: nsec).data;
      if (privHex.isEmpty) {
        throw const FormatException('empty private key');
      }
      return privHex;
    } catch (_) {
      throw const FormatException('failed to decode private key');
    }
  }

  static String _derivePubkey(String privHex) {
    try {
      return nostr.Keys(privHex).public;
    } catch (_) {
      throw const FormatException('failed to derive pubkey');
    }
  }

  AgentUsageConnectionState _connectionForSession(SessionStatus status) {
    if (_errorMessage != null && _unsubscribe == null && _startFuture == null) {
      return AgentUsageConnectionState.error;
    }
    return switch (status) {
      SessionStatus.connected =>
        _unsubscribe == null
            ? AgentUsageConnectionState.connecting
            : AgentUsageConnectionState.open,
      SessionStatus.connecting ||
      SessionStatus.reconnecting => AgentUsageConnectionState.connecting,
      SessionStatus.disconnected => AgentUsageConnectionState.idle,
    };
  }

  static String _usageErrorMessage(Object error) {
    if (error is FormatException) {
      return error.message;
    }
    return 'Agent usage subscription failed: $error';
  }
}

final agentUsageRelayProvider =
    NotifierProvider<AgentUsageRelayNotifier, AgentUsageRelayState>(
      AgentUsageRelayNotifier.new,
    );

/// Per-agent view over [agentUsageRelayProvider], for widgets that only need
/// one agent's snapshot.
final agentUsageSnapshotProvider = Provider.family<AgentUsageSnapshot?, String>(
  (ref, agentPubkey) {
    final relayState = ref.watch(agentUsageRelayProvider);
    return relayState.snapshotsByAgent[agentPubkey.toLowerCase()];
  },
);
