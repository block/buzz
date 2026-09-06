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
  final String? errorMessage;

  const AgentUsageRelayState({
    required this.connection,
    required this.snapshotsByAgent,
    this.errorMessage,
  });

  const AgentUsageRelayState.initial()
    : connection = AgentUsageConnectionState.idle,
      snapshotsByAgent = const {},
      errorMessage = null;
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
    final identityKey = '${config.baseUrl}|${config.nsec ?? ''}';

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
      errorMessage: _errorMessage,
    );
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

  void _handleEvent(NostrEvent event) {
    // Relays are untrusted. Verify the canonical event id and Schnorr
    // signature before deduplication so a forged duplicate cannot poison the
    // recent-id cache and suppress a later valid event.
    if (!_hasValidNip01Signature(event)) {
      return;
    }

    if (!_recentEventIdSet.add(event.id)) {
      return;
    }
    _recentEventIds.add(event.id);
    if (_recentEventIds.length > _maxRecentEventIds) {
      final removed = _recentEventIds.removeAt(0);
      _recentEventIdSet.remove(removed);
    }

    // NIP-AM: events MUST carry exactly one `p` tag (owner) and one `agent`
    // tag equal to `pubkey`. Tag matching alone is not an authorization
    // decision here (the relay already gates reads to the owner), but a
    // malformed envelope cannot be decrypted meaningfully either way.
    final agentPubkey = event.getTagValue('agent');
    if (agentPubkey == null ||
        event.pubkey.toLowerCase() != agentPubkey.toLowerCase()) {
      return;
    }

    final ownerPubkey = _ownerPubkey;
    final privHex = _privHex;
    if (ownerPubkey == null || privHex == null) {
      return;
    }

    final pTag = event.getTagValue('p');
    if (pTag?.toLowerCase() != ownerPubkey.toLowerCase()) {
      return;
    }

    final normalizedAgent = agentPubkey.toLowerCase();
    final payload = _decryptPayload(event, normalizedAgent, privHex);
    if (payload == null) return;

    final merged = mergeAgentUsageSnapshot(
      _snapshotsByAgent[normalizedAgent],
      payload,
    );
    _snapshotsByAgent[normalizedAgent] = merged;
    _emit(connection: AgentUsageConnectionState.open);
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
      final conversationKey = _conversationKeysByAgent.putIfAbsent(
        normalizedAgent,
        () => getConversationKey(privHex, normalizedAgent),
      );
      final plaintext = nip44Decrypt(conversationKey, event.content);
      final json = jsonDecode(plaintext) as Map<String, dynamic>;
      return AgentTurnMetricPayload.fromJson(json);
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
      errorMessage: _errorMessage,
    );
  }

  Map<String, AgentUsageSnapshot> _snapshotFrames() {
    return Map<String, AgentUsageSnapshot>.unmodifiable(_snapshotsByAgent);
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
