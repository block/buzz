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
  final String? sessionId;
  final String? turnId;
  final dynamic payload;

  const ObserverFrame({
    required this.seq,
    required this.timestamp,
    required this.kind,
    this.agentIndex,
    this.channelId,
    this.sessionId,
    this.turnId,
    this.payload,
  });

  factory ObserverFrame.fromJson(Map<String, dynamic> json) => ObserverFrame(
    seq: json['seq'] as int? ?? 0,
    timestamp: json['timestamp'] as String? ?? '',
    kind: json['kind'] as String? ?? '',
    agentIndex: json['agentIndex'] as int?,
    channelId: json['channelId'] as String?,
    sessionId: json['sessionId'] as String?,
    turnId: json['turnId'] as String?,
    payload: json['payload'],
  );
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
