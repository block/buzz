import 'dart:convert';

import 'package:flutter/foundation.dart';

/// How stale [AgentUsageSnapshot.lastEventAt] must be before the compact
/// indicator stops presenting cached data as current.
const staleAfter = Duration(minutes: 45);

int _utf8Length(String value) => utf8.encode(value).length;

int? _optionalTokenCount(Map<String, dynamic> json, String field) {
  if (!json.containsKey(field)) return null;
  final value = json[field];
  if (value == null) return null;
  if (value is! int || value < 0) {
    throw FormatException('$field must be a non-negative integer');
  }
  return value;
}

double? _optionalCost(Map<String, dynamic> json, String field) {
  if (!json.containsKey(field)) return null;
  final value = json[field];
  if (value == null) return null;
  if (value is! num) {
    throw FormatException('$field must be a non-negative finite number');
  }
  final parsed = value.toDouble();
  if (!parsed.isFinite || parsed < 0) {
    throw FormatException('$field must be a non-negative finite number');
  }
  return parsed;
}

DateTime _parseRfc3339(Object? value, String field) {
  if (value is! String || !RegExp(r'(?:Z|[+-]\d{2}:\d{2})$').hasMatch(value)) {
    throw FormatException('$field must be RFC 3339');
  }
  final parsed = DateTime.tryParse(value);
  if (parsed == null) throw FormatException('$field must be RFC 3339');
  return parsed;
}

/// Token-usage counters for one measurement window (a turn or a session
/// cumulative), decoded from a NIP-AM `kind:44200` payload. Null fields mean
/// the harness did not report them — NOT that the count was zero.
@immutable
class AgentTokenCounts {
  final int? inputTokens;
  final int? outputTokens;
  final int? totalTokens;
  final double? costUsd;
  final int? cacheReadTokens;
  final int? cacheWriteTokens;

  const AgentTokenCounts({
    this.inputTokens,
    this.outputTokens,
    this.totalTokens,
    this.costUsd,
    this.cacheReadTokens,
    this.cacheWriteTokens,
  });

  /// Throws [FormatException] for a non-finite or negative `costUsd`,
  /// mirroring NIP-AM §Numeric validity (`buzz-core`'s
  /// `AgentTurnMetricPayload::validate`).
  factory AgentTokenCounts.fromJson(Map<String, dynamic> json) {
    return AgentTokenCounts(
      inputTokens: _optionalTokenCount(json, 'inputTokens'),
      outputTokens: _optionalTokenCount(json, 'outputTokens'),
      totalTokens: _optionalTokenCount(json, 'totalTokens'),
      costUsd: _optionalCost(json, 'costUsd'),
      cacheReadTokens: _optionalTokenCount(json, 'cacheReadTokens'),
      cacheWriteTokens: _optionalTokenCount(json, 'cacheWriteTokens'),
    );
  }

  bool get hasObservation =>
      inputTokens != null ||
      outputTokens != null ||
      totalTokens != null ||
      costUsd != null ||
      cacheReadTokens != null ||
      cacheWriteTokens != null;
}

/// A provider-account quota window (`accountUsageWindows` entry), e.g.
/// `{"label": "Session", "usedPercent": 12.5, "resetAt": "..."}`.
@immutable
class AgentUsageWindow {
  final String label;
  final double usedPercent;
  final DateTime? resetAt;

  const AgentUsageWindow({
    required this.label,
    required this.usedPercent,
    this.resetAt,
  });

  /// Throws [FormatException] on a missing label or a non-finite/negative
  /// `usedPercent`, matching the NIP-AM validity contract.
  factory AgentUsageWindow.fromJson(Map<String, dynamic> json) {
    final label = json['label'] as String?;
    if (label == null || label.trim().isEmpty || _utf8Length(label) > 128) {
      throw const FormatException('accountUsageWindows entry missing label');
    }
    final usedPercentRaw = json['usedPercent'];
    final usedPercent = usedPercentRaw is num
        ? usedPercentRaw.toDouble()
        : double.nan;
    if (!usedPercent.isFinite || usedPercent < 0) {
      throw const FormatException(
        'accountUsageWindows.usedPercent must be finite and non-negative',
      );
    }
    final resetAt = json.containsKey('resetAt')
        ? _parseRfc3339(json['resetAt'], 'accountUsageWindows.resetAt')
        : null;
    return AgentUsageWindow(
      label: label,
      usedPercent: usedPercent,
      resetAt: resetAt,
    );
  }
}

/// Decrypted payload of a `kind:44200` NIP-AM Agent Turn Metric event.
/// Models only the fields the compact usage display consumes; unrecognized
/// JSON fields are ignored per the NIP's forward-compatibility contract.
@immutable
class AgentTurnMetricPayload {
  final String harness;
  final String? model;
  final String? channelId;
  final String? threadRootId;
  final DateTime timestamp;
  final AgentTokenCounts? cumulative;
  final int? contextUsedTokens;
  final int? contextLimitTokens;
  final List<AgentUsageWindow> accountUsageWindows;

  const AgentTurnMetricPayload({
    required this.harness,
    this.model,
    this.channelId,
    this.threadRootId,
    required this.timestamp,
    this.cumulative,
    this.contextUsedTokens,
    this.contextLimitTokens,
    this.accountUsageWindows = const [],
  });

  /// Throws [FormatException] when required fields are missing or any
  /// numeric field fails NIP-AM validity — callers MUST drop the whole event
  /// on failure rather than partially ingest it (NIP-AM Client Behavior:
  /// "ignore events that fail to decrypt or parse").
  factory AgentTurnMetricPayload.fromJson(Map<String, dynamic> json) {
    final harness = json['harness'] as String?;
    if (harness == null ||
        harness.trim().isEmpty ||
        _utf8Length(harness) > 128) {
      throw const FormatException('agent turn metric missing harness');
    }
    final timestamp = _parseRfc3339(json['timestamp'], 'timestamp');

    final model = json['model'] as String?;
    final channelId = json['channelId'] as String?;
    final threadRootId = json['threadRootId'] as String?;
    for (final field in [
      ('model', model),
      ('channelId', channelId),
      ('sessionId', json['sessionId'] as String?),
      ('turnId', json['turnId'] as String?),
    ]) {
      final value = field.$2;
      if (value != null && (value.trim().isEmpty || _utf8Length(value) > 256)) {
        throw FormatException(
          '${field.$1} must be non-empty and at most 256 bytes',
        );
      }
    }
    if (threadRootId != null &&
        (channelId == null ||
            !RegExp(r'^[0-9a-f]{64}$').hasMatch(threadRootId))) {
      throw const FormatException(
        'threadRootId requires channelId and 64 lowercase hex characters',
      );
    }

    final contextUsedTokens = _optionalTokenCount(json, 'contextUsedTokens');
    final contextLimitTokens = _optionalTokenCount(json, 'contextLimitTokens');
    if ((contextUsedTokens == null) != (contextLimitTokens == null) ||
        (contextUsedTokens != null &&
            (contextUsedTokens == 0 || contextLimitTokens == 0))) {
      throw const FormatException(
        'contextUsedTokens and contextLimitTokens must be a complete positive pair',
      );
    }

    final windowsRaw = json['accountUsageWindows'];
    if (windowsRaw != null && windowsRaw is! List) {
      throw const FormatException('accountUsageWindows must be an array');
    }
    final windows = <AgentUsageWindow>[];
    for (final entry in windowsRaw as List? ?? const []) {
      if (entry is! Map<String, dynamic>) {
        throw const FormatException(
          'accountUsageWindows entries must be objects',
        );
      }
      windows.add(AgentUsageWindow.fromJson(entry));
    }
    if (windows.length > 64) {
      throw const FormatException(
        'accountUsageWindows must contain at most 64 entries',
      );
    }

    final turnRaw = json['turn'];
    if (turnRaw != null && turnRaw is! Map<String, dynamic>) {
      throw const FormatException('turn must be an object');
    }
    final turn = turnRaw is Map<String, dynamic>
        ? AgentTokenCounts.fromJson(turnRaw)
        : null;

    final cumulativeRaw = json['cumulative'];
    if (cumulativeRaw != null && cumulativeRaw is! Map<String, dynamic>) {
      throw const FormatException('cumulative must be an object');
    }
    final cumulative = cumulativeRaw is Map<String, dynamic>
        ? AgentTokenCounts.fromJson(cumulativeRaw)
        : null;
    if (cumulative != null) {
      final sessionId = json['sessionId'];
      final turnSeq = json['turnSeq'];
      if (sessionId is! String ||
          sessionId.trim().isEmpty ||
          turnSeq is! int ||
          turnSeq < 0) {
        throw const FormatException(
          'sessionId and turnSeq are required with cumulative',
        );
      }
    }

    final pricingRaw = json['pricingIdentity'];
    if (pricingRaw != null) {
      if (pricingRaw is! Map<String, dynamic>) {
        throw const FormatException('pricingIdentity must be an object');
      }
      final authority = pricingRaw['authority'];
      final pricingModel = pricingRaw['model'];
      const authorities = {
        'api.anthropic.com',
        'api.openai.com',
        'openrouter.ai',
      };
      if (authority is! String ||
          !authorities.contains(authority) ||
          pricingModel is! String ||
          pricingModel.trim().isEmpty ||
          _utf8Length(pricingModel) > 256) {
        throw const FormatException('invalid pricingIdentity');
      }
      final cacheClass = pricingRaw['cacheClass'];
      if (cacheClass != null &&
          (cacheClass is! String ||
              cacheClass.trim().isEmpty ||
              _utf8Length(cacheClass) > 256)) {
        throw const FormatException('invalid pricingIdentity.cacheClass');
      }
    }

    if (turn?.hasObservation != true &&
        cumulative?.hasObservation != true &&
        contextUsedTokens == null &&
        windows.isEmpty) {
      throw const FormatException('agent turn metric contains no observation');
    }

    return AgentTurnMetricPayload(
      harness: harness,
      model: model,
      channelId: channelId,
      threadRootId: threadRootId,
      timestamp: timestamp,
      cumulative: cumulative,
      contextUsedTokens: contextUsedTokens,
      contextLimitTokens: contextLimitTokens,
      accountUsageWindows: List.unmodifiable(windows),
    );
  }
}

/// Merged latest-known usage state for one agent, built by folding a stream
/// of [AgentTurnMetricPayload]s field-by-field (see [mergeAgentUsageSnapshot]).
/// Each field tracks its own "as of" timestamp because a producer may report
/// some fields on one turn and omit them on the next — the display always
/// wants the newest *known* value per field, not just the newest event.
@immutable
class AgentUsageSnapshot {
  final String harness;
  final String? model;
  final DateTime lastEventAt;
  final String lastEventId;

  final int? contextUsedTokens;
  final int? contextLimitTokens;
  final DateTime? contextSnapshotAt;
  final String? contextSnapshotEventId;

  final List<AgentUsageWindow> accountUsageWindows;
  final DateTime? accountUsageWindowsAt;
  final String? accountUsageWindowsEventId;

  final AgentTokenCounts? cumulative;
  final DateTime? cumulativeAt;
  final String? cumulativeEventId;

  const AgentUsageSnapshot({
    required this.harness,
    this.model,
    required this.lastEventAt,
    this.lastEventId = '',
    this.contextUsedTokens,
    this.contextLimitTokens,
    this.contextSnapshotAt,
    this.contextSnapshotEventId,
    this.accountUsageWindows = const [],
    this.accountUsageWindowsAt,
    this.accountUsageWindowsEventId,
    this.cumulative,
    this.cumulativeAt,
    this.cumulativeEventId,
  });

  /// Fraction (0.0–1.0) of context window consumed, clamped for display.
  /// `contextUsedTokens` MAY exceed `contextLimitTokens` on the wire (NIP-AM);
  /// callers render the clamped bar but keep the raw values for the detail
  /// view.
  double? get contextUsedFraction {
    final used = contextUsedTokens;
    final limit = contextLimitTokens;
    if (used == null || limit == null || limit <= 0) return null;
    return (used / limit).clamp(0.0, 1.0);
  }
}

/// Folds [payload] (decrypted from an event with payload-declared time
/// [payload.timestamp]) into [previous], keeping each field's newest known
/// value independently.
///
/// A field is only overwritten when [payload] actually reports it (NIP-AM:
/// omission means "not reported", never "cleared") AND [payload.timestamp] is
/// not older than that field's current "as of" time — out-of-order delivery
/// (relay replay, backfill racing the live subscription) must not let a
/// stale event clobber newer data.
AgentUsageSnapshot mergeAgentUsageSnapshot(
  AgentUsageSnapshot? previous,
  AgentTurnMetricPayload payload, {
  String eventId = '',
}) {
  final useAsLatest = _isNewerObservation(
    payload.timestamp,
    eventId,
    previous?.lastEventAt,
    previous?.lastEventId,
  );
  final lastEventAt = useAsLatest ? payload.timestamp : previous!.lastEventAt;
  final lastEventId = useAsLatest ? eventId : previous!.lastEventId;

  final useContext =
      payload.contextUsedTokens != null &&
      payload.contextLimitTokens != null &&
      _isNewerObservation(
        payload.timestamp,
        eventId,
        previous?.contextSnapshotAt,
        previous?.contextSnapshotEventId,
      );

  final useWindows =
      payload.accountUsageWindows.isNotEmpty &&
      _isNewerObservation(
        payload.timestamp,
        eventId,
        previous?.accountUsageWindowsAt,
        previous?.accountUsageWindowsEventId,
      );

  final useCumulative =
      payload.cumulative != null &&
      _isNewerObservation(
        payload.timestamp,
        eventId,
        previous?.cumulativeAt,
        previous?.cumulativeEventId,
      );

  // harness/model are descriptive of the publishing agent process; take them
  // from whichever event is currently newest overall.
  final useDescriptive = useAsLatest;

  return AgentUsageSnapshot(
    harness: useDescriptive ? payload.harness : previous!.harness,
    model: useDescriptive
        ? (payload.model ?? previous?.model)
        : previous!.model,
    lastEventAt: lastEventAt,
    lastEventId: lastEventId,
    contextUsedTokens: useContext
        ? payload.contextUsedTokens
        : previous?.contextUsedTokens,
    contextLimitTokens: useContext
        ? payload.contextLimitTokens
        : previous?.contextLimitTokens,
    contextSnapshotAt: useContext
        ? payload.timestamp
        : previous?.contextSnapshotAt,
    contextSnapshotEventId: useContext
        ? eventId
        : previous?.contextSnapshotEventId,
    accountUsageWindows: useWindows
        ? payload.accountUsageWindows
        : (previous?.accountUsageWindows ?? const []),
    accountUsageWindowsAt: useWindows
        ? payload.timestamp
        : previous?.accountUsageWindowsAt,
    accountUsageWindowsEventId: useWindows
        ? eventId
        : previous?.accountUsageWindowsEventId,
    cumulative: useCumulative ? payload.cumulative : previous?.cumulative,
    cumulativeAt: useCumulative ? payload.timestamp : previous?.cumulativeAt,
    cumulativeEventId: useCumulative ? eventId : previous?.cumulativeEventId,
  );
}

bool _isNewerObservation(
  DateTime candidateAt,
  String candidateEventId,
  DateTime? currentAt,
  String? currentEventId,
) {
  if (currentAt == null) return true;
  final timeOrder = candidateAt.compareTo(currentAt);
  if (timeOrder != 0) return timeOrder > 0;
  return candidateEventId.compareTo(currentEventId ?? '') > 0;
}

/// Display status for the compact usage indicator.
enum AgentUsageStatus {
  /// No usage event has ever been observed for this agent yet.
  unknown,

  /// An event exists, but it contains no context or account-quota observation.
  unavailable,

  /// A usage snapshot exists and is recent.
  fresh,

  /// A usage snapshot exists but is older than [staleAfter].
  stale,

  /// No usage snapshot exists and the live subscription is not healthy, so
  /// none is expected soon.
  error,
}

({double fraction, DateTime timestamp})? _primaryUsageMetric(
  AgentUsageSnapshot snapshot,
) {
  final known = <({double fraction, DateTime timestamp})>[
    ...snapshot.accountUsageWindows.map(
      (window) => (
        fraction: (window.usedPercent / 100).clamp(0.0, 1.0).toDouble(),
        timestamp: snapshot.accountUsageWindowsAt ?? snapshot.lastEventAt,
      ),
    ),
    if (snapshot.contextUsedFraction != null)
      (
        fraction: snapshot.contextUsedFraction!,
        timestamp: snapshot.contextSnapshotAt ?? snapshot.lastEventAt,
      ),
  ];
  if (known.isEmpty) return null;
  return known.reduce(
    (highest, candidate) =>
        candidate.fraction > highest.fraction ? candidate : highest,
  );
}

AgentUsageStatus agentUsageStatusFor(
  AgentUsageSnapshot? snapshot, {
  required bool subscriptionErrored,
  DateTime? now,
}) {
  if (snapshot == null) {
    return subscriptionErrored
        ? AgentUsageStatus.error
        : AgentUsageStatus.unknown;
  }
  if (_primaryUsageMetric(snapshot) == null) {
    return AgentUsageStatus.unavailable;
  }
  final effectiveNow = now ?? DateTime.now();
  final statusTimestamp =
      _primaryUsageMetric(snapshot)?.timestamp ?? snapshot.lastEventAt;
  if (effectiveNow.difference(statusTimestamp) > staleAfter) {
    return AgentUsageStatus.stale;
  }
  return AgentUsageStatus.fresh;
}

/// The single most urgent usage fraction (0.0–1.0) to surface on the compact
/// ring: the highest known provider-account or context-window usage, or null
/// when the publisher did not report either.
double? primaryUsageFraction(AgentUsageSnapshot? snapshot) {
  if (snapshot == null) return null;
  return _primaryUsageMetric(snapshot)?.fraction;
}
