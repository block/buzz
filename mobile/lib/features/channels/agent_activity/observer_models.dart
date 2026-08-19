import 'package:flutter/foundation.dart';

/// Connection state for the observer relay subscription.
enum ObserverConnectionState { idle, connecting, open, error }

/// Status of a tool execution.
enum ToolStatus { executing, completed, failed, pending }

/// A decrypted observer frame from a kind:24200 event.
@immutable
class ObserverFrame {
  final int seq;
  final String timestamp;
  final String kind;
  final int? agentIndex;
  final String? channelId;
  final String? threadHeadId;
  final bool hasThreadScope;
  final String? sessionId;
  final String? turnId;
  final String? startedAt;
  final DateTime? receivedAt;
  final dynamic payload;

  const ObserverFrame({
    required this.seq,
    required this.timestamp,
    required this.kind,
    this.agentIndex,
    this.channelId,
    this.threadHeadId,
    bool? hasThreadScope,
    this.sessionId,
    this.turnId,
    this.startedAt,
    this.receivedAt,
    this.payload,
  }) : hasThreadScope = hasThreadScope ?? threadHeadId != null;

  factory ObserverFrame.fromJson(
    Map<String, dynamic> json, {
    DateTime? receivedAt,
  }) => ObserverFrame(
    seq: json['seq'] as int? ?? 0,
    timestamp: json['timestamp'] as String? ?? '',
    kind: json['kind'] as String? ?? '',
    agentIndex: json['agentIndex'] as int?,
    channelId: json['channelId'] as String?,
    threadHeadId: json['threadHeadId'] as String?,
    hasThreadScope: json.containsKey('threadHeadId'),
    sessionId: json['sessionId'] as String?,
    turnId: json['turnId'] as String?,
    startedAt: json['startedAt'] as String?,
    receivedAt: receivedAt,
    payload: json['payload'],
  );
}

/// Orders observer frames by their protocol stream before presentation time.
///
/// NIP-AO guarantees that [ObserverFrame.seq] increases within one session.
/// Host wall-clock timestamps are only presentation metadata and may jump in
/// either direction. Frames without a session inherit one from another frame
/// for the same turn; unrelated streams use local receipt time to establish
/// their relative epoch.
List<ObserverFrame> orderObserverFrames(Iterable<ObserverFrame> frames) {
  final ordered = [...frames];
  final sessionByTurn = <String, String>{};
  for (final frame in ordered) {
    final sessionId = frame.sessionId;
    final turnId = frame.turnId;
    if (sessionId != null &&
        sessionId.isNotEmpty &&
        turnId != null &&
        turnId.isNotEmpty) {
      sessionByTurn[turnId] = sessionId;
    }
  }

  String streamKey(ObserverFrame frame) {
    final sessionId = frame.sessionId;
    if (sessionId != null && sessionId.isNotEmpty) return 'session:$sessionId';
    final turnId = frame.turnId;
    if (turnId != null && turnId.isNotEmpty) {
      final inheritedSession = sessionByTurn[turnId];
      return inheritedSession == null
          ? 'turn:$turnId'
          : 'session:$inheritedSession';
    }
    return 'legacy';
  }

  DateTime presentationTime(ObserverFrame frame) =>
      frame.receivedAt ??
      DateTime.tryParse(frame.timestamp)?.toUtc() ??
      DateTime.fromMillisecondsSinceEpoch(frame.seq, isUtc: true);

  final epochByStream = <String, DateTime>{};
  for (final frame in ordered) {
    final key = streamKey(frame);
    final time = presentationTime(frame);
    final current = epochByStream[key];
    if (current == null || time.isBefore(current)) epochByStream[key] = time;
  }

  ordered.sort((a, b) {
    final aKey = streamKey(a);
    final bKey = streamKey(b);
    if (aKey == bKey) {
      final sequence = a.seq.compareTo(b.seq);
      if (sequence != 0) return sequence;
      return presentationTime(a).compareTo(presentationTime(b));
    }
    final epoch = epochByStream[aKey]!.compareTo(epochByStream[bKey]!);
    if (epoch != 0) return epoch;
    return aKey.compareTo(bKey);
  });
  return ordered;
}

/// A section within prompt context metadata.
@immutable
class PromptSection {
  final String title;
  final String body;

  const PromptSection({required this.title, required this.body});
}

/// A single item in the agent activity transcript.
sealed class TranscriptItem {
  String get id;
  String get timestamp;
}

class MessageItem extends TranscriptItem {
  @override
  final String id;
  final String role;
  final String title;
  String text;
  @override
  final String timestamp;

  MessageItem({
    required this.id,
    required this.role,
    required this.title,
    required this.text,
    required this.timestamp,
  });
}

class ThoughtItem extends TranscriptItem {
  @override
  final String id;
  final String title;
  String text;
  @override
  final String timestamp;

  ThoughtItem({
    required this.id,
    required this.title,
    required this.text,
    required this.timestamp,
  });
}

class LifecycleItem extends TranscriptItem {
  @override
  final String id;
  final String title;
  String text;
  @override
  final String timestamp;

  LifecycleItem({
    required this.id,
    required this.title,
    required this.text,
    required this.timestamp,
  });
}

class MetadataItem extends TranscriptItem {
  @override
  final String id;
  final String title;
  List<PromptSection> sections;
  @override
  final String timestamp;

  MetadataItem({
    required this.id,
    required this.title,
    required this.sections,
    required this.timestamp,
  });
}

class ToolItem extends TranscriptItem {
  @override
  final String id;
  String title;
  String toolName;
  String? buzzToolName;
  ToolStatus status;
  Map<String, dynamic> args;
  String result;
  bool isError;
  @override
  final String timestamp;

  ToolItem({
    required this.id,
    required this.title,
    required this.toolName,
    this.buzzToolName,
    required this.status,
    required this.args,
    required this.result,
    required this.isError,
    required this.timestamp,
  });
}
