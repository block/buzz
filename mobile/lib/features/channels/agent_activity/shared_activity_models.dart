import 'dart:collection';
import 'dart:convert';

import 'package:flutter/foundation.dart';

const sharedActivityFrameVersion = 1;
const sharedActivityMaxFrameBytes = 4096;
const sharedActivityMaxFrameItems = 32;
const sharedActivityMaxDurationMs = 604800000;
const sharedActivityMaxTokenCount = 1000000000000;

enum SharedActivityClass { turn, tool, usage }

enum SharedActivityStatus {
  started,
  pending,
  running,
  completed,
  failed,
  cancelled,
}

enum SharedActivityToolKind {
  read,
  edit,
  delete,
  move,
  search,
  execute,
  think,
  fetch,
  switchMode,
  other,
}

@immutable
class SharedActivityUsage {
  final int? inputTokens;
  final int? outputTokens;
  final int? totalTokens;
  final int? cacheReadTokens;
  final int? cacheWriteTokens;

  const SharedActivityUsage({
    this.inputTokens,
    this.outputTokens,
    this.totalTokens,
    this.cacheReadTokens,
    this.cacheWriteTokens,
  });
}

@immutable
class SharedActivity {
  final String activityId;
  final DateTime occurredAt;
  final SharedActivityClass activityClass;
  final SharedActivityStatus status;
  final SharedActivityToolKind? toolKind;
  final int? durationMs;
  final SharedActivityUsage? usage;

  const SharedActivity({
    required this.activityId,
    required this.occurredAt,
    required this.activityClass,
    required this.status,
    this.toolKind,
    this.durationMs,
    this.usage,
  });
}

/// Parses the privacy-sanitized kind:24201 payload.
///
/// This parser intentionally has no dependency on the owner-only observer
/// payloads. Every object is a closed schema: unknown fields are rejected
/// rather than ignored.
List<SharedActivity> parseSharedActivityFrame(String content) {
  if (utf8.encode(content).length > sharedActivityMaxFrameBytes) {
    throw const FormatException('shared activity frame is too large');
  }

  final Object? decoded = jsonDecode(content);
  final frame = _stringMap(decoded, 'frame');
  _requireExactKeys(frame, const {'version', 'activities'}, const {}, 'frame');
  if (frame['version'] != sharedActivityFrameVersion) {
    throw const FormatException('unsupported shared activity version');
  }

  final activities = frame['activities'];
  if (activities is! List ||
      activities.isEmpty ||
      activities.length > sharedActivityMaxFrameItems) {
    throw const FormatException('activities must contain 1 to 32 items');
  }

  return List<SharedActivity>.unmodifiable(
    activities.map((value) => _parseActivity(_stringMap(value, 'activity'))),
  );
}

SharedActivity _parseActivity(Map<String, Object?> value) {
  _requireExactKeys(
    value,
    const {'activityId', 'occurredAt', 'activityClass', 'status'},
    const {'toolKind', 'durationMs', 'usage'},
    'activity',
  );

  final activityId = _requiredString(value, 'activityId');
  if (!_uuidPattern.hasMatch(activityId)) {
    throw const FormatException('activityId must be a UUID');
  }

  final occurredAtValue = _requiredString(value, 'occurredAt');
  if (!_rfc3339Pattern.hasMatch(occurredAtValue)) {
    throw const FormatException('occurredAt must be RFC3339');
  }
  final occurredAt = DateTime.tryParse(occurredAtValue);
  if (occurredAt == null) {
    throw const FormatException('occurredAt must be a valid timestamp');
  }

  final activityClass = _parseActivityClass(
    _requiredString(value, 'activityClass'),
  );
  final status = _parseStatus(_requiredString(value, 'status'));
  final toolKind = value.containsKey('toolKind')
      ? _parseToolKind(_requiredString(value, 'toolKind'))
      : null;
  final durationMs = value.containsKey('durationMs')
      ? _boundedInt(
          value['durationMs'],
          'durationMs',
          sharedActivityMaxDurationMs,
        )
      : null;
  final usage = value.containsKey('usage')
      ? _parseUsage(_stringMap(value['usage'], 'usage'))
      : null;

  if (durationMs != null && !_terminalStatuses.contains(status)) {
    throw const FormatException(
      'durationMs is allowed only for terminal statuses',
    );
  }

  switch (activityClass) {
    case SharedActivityClass.turn:
      if (toolKind != null ||
          usage != null ||
          status == SharedActivityStatus.pending) {
        throw const FormatException('invalid turn activity');
      }
    case SharedActivityClass.tool:
      if (toolKind == null ||
          usage != null ||
          status == SharedActivityStatus.started) {
        throw const FormatException('invalid tool activity');
      }
    case SharedActivityClass.usage:
      if (status != SharedActivityStatus.completed ||
          toolKind != null ||
          durationMs != null ||
          usage == null) {
        throw const FormatException('invalid usage activity');
      }
  }

  return SharedActivity(
    activityId: activityId,
    occurredAt: occurredAt.toUtc(),
    activityClass: activityClass,
    status: status,
    toolKind: toolKind,
    durationMs: durationMs,
    usage: usage,
  );
}

SharedActivityUsage _parseUsage(Map<String, Object?> value) {
  const fields = {
    'inputTokens',
    'outputTokens',
    'totalTokens',
    'cacheReadTokens',
    'cacheWriteTokens',
  };
  _requireExactKeys(value, const {}, fields, 'usage');
  if (value.isEmpty) {
    throw const FormatException('usage requires a token count');
  }

  int? count(String field) => value.containsKey(field)
      ? _boundedInt(value[field], field, sharedActivityMaxTokenCount)
      : null;

  return SharedActivityUsage(
    inputTokens: count('inputTokens'),
    outputTokens: count('outputTokens'),
    totalTokens: count('totalTokens'),
    cacheReadTokens: count('cacheReadTokens'),
    cacheWriteTokens: count('cacheWriteTokens'),
  );
}

SharedActivityClass _parseActivityClass(String value) => switch (value) {
  'turn' => SharedActivityClass.turn,
  'tool' => SharedActivityClass.tool,
  'usage' => SharedActivityClass.usage,
  _ => throw const FormatException('unknown activityClass'),
};

SharedActivityStatus _parseStatus(String value) => switch (value) {
  'started' => SharedActivityStatus.started,
  'pending' => SharedActivityStatus.pending,
  'running' => SharedActivityStatus.running,
  'completed' => SharedActivityStatus.completed,
  'failed' => SharedActivityStatus.failed,
  'cancelled' => SharedActivityStatus.cancelled,
  _ => throw const FormatException('unknown status'),
};

SharedActivityToolKind _parseToolKind(String value) => switch (value) {
  'read' => SharedActivityToolKind.read,
  'edit' => SharedActivityToolKind.edit,
  'delete' => SharedActivityToolKind.delete,
  'move' => SharedActivityToolKind.move,
  'search' => SharedActivityToolKind.search,
  'execute' => SharedActivityToolKind.execute,
  'think' => SharedActivityToolKind.think,
  'fetch' => SharedActivityToolKind.fetch,
  'switch_mode' => SharedActivityToolKind.switchMode,
  'other' => SharedActivityToolKind.other,
  _ => throw const FormatException('unknown toolKind'),
};

Map<String, Object?> _stringMap(Object? value, String name) {
  if (value is! Map<String, dynamic>) {
    throw FormatException('$name must be an object');
  }
  return value;
}

void _requireExactKeys(
  Map<String, Object?> value,
  Set<String> required,
  Set<String> optional,
  String name,
) {
  if (!value.keys.toSet().containsAll(required) ||
      value.keys.any(
        (key) => !required.contains(key) && !optional.contains(key),
      )) {
    throw FormatException('$name has missing or unknown fields');
  }
}

String _requiredString(Map<String, Object?> value, String field) {
  final result = value[field];
  if (result is! String || result.isEmpty) {
    throw FormatException('$field must be a non-empty string');
  }
  return result;
}

int _boundedInt(Object? value, String field, int max) {
  if (value is! int || value < 0 || value > max) {
    throw FormatException('$field is outside its allowed range');
  }
  return value;
}

final _uuidPattern = RegExp(
  r'^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$',
);
final _rfc3339Pattern = RegExp(
  r'^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:\d{2})$',
);
const _terminalStatuses = {
  SharedActivityStatus.completed,
  SharedActivityStatus.failed,
  SharedActivityStatus.cancelled,
};

/// Bounded, activity-ID-deduplicating storage for shared activity updates.
class SharedActivityStore {
  final int maxItems;
  final LinkedHashMap<String, SharedActivity> _itemsById = LinkedHashMap();

  SharedActivityStore({this.maxItems = 200})
    : assert(maxItems > 0, 'maxItems must be positive');

  List<SharedActivity> get items => List.unmodifiable(_itemsById.values);

  void addAll(Iterable<SharedActivity> items) {
    for (final item in items) {
      _itemsById[item.activityId] = item;
    }

    final ordered = _itemsById.values.toList()
      ..sort((left, right) {
        final time = left.occurredAt.compareTo(right.occurredAt);
        return time != 0 ? time : left.activityId.compareTo(right.activityId);
      });
    final retained = ordered.length > maxItems
        ? ordered.sublist(ordered.length - maxItems)
        : ordered;
    _itemsById
      ..clear()
      ..addEntries(retained.map((item) => MapEntry(item.activityId, item)));
  }

  void clear() => _itemsById.clear();
}
