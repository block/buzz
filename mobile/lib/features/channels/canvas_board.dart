import 'dart:convert';

import 'package:flutter/foundation.dart';

enum CanvasBoardCardType {
  agent,
  artifact,
  conversation,
  decision,
  note,
  person,
  project,
  task,
}

enum CanvasBoardCardStatus { backlog, doing, done }

enum ChannelCanvasView { board, stream }

@immutable
class CanvasBoardCard {
  final String id;
  final String title;
  final String body;
  final CanvasBoardCardType type;
  final CanvasBoardCardStatus status;
  final String? threadId;
  final String? authorPubkey;

  const CanvasBoardCard({
    required this.id,
    required this.title,
    required this.body,
    required this.type,
    required this.status,
    this.threadId,
    this.authorPubkey,
  });
}

@immutable
class CanvasBoard {
  final String? title;
  final String introduction;
  final List<CanvasBoardCard> cards;

  const CanvasBoard({
    required this.title,
    required this.introduction,
    required this.cards,
  });
}

const channelCanvasViewPreferencePrefix = 'buzz.channelViewMode.v1';

String channelCanvasViewPreferenceKey(String channelId) =>
    '$channelCanvasViewPreferencePrefix.$channelId';

bool channelHasCanvasBoard({
  required String channelName,
  required bool isDm,
  required String? content,
}) {
  if (isDm) return false;
  final canonicalName = channelName.replaceFirst(RegExp(r'^[#\s]+'), '').trim();
  return content?.trim().isNotEmpty == true ||
      canonicalName.toLowerCase() == 'dispatch';
}

ChannelCanvasView initialChannelCanvasView({
  required String channelName,
  required String? storedValue,
}) {
  if (storedValue == ChannelCanvasView.board.name) {
    return ChannelCanvasView.board;
  }
  if (storedValue == ChannelCanvasView.stream.name) {
    return ChannelCanvasView.stream;
  }
  final canonicalName = channelName.replaceFirst(RegExp(r'^[#\s]+'), '').trim();
  return canonicalName.toLowerCase() == 'dispatch'
      ? ChannelCanvasView.board
      : ChannelCanvasView.stream;
}

final _h1Pattern = RegExp(r'^#\s+(.+?)\s*#*\s*$');
final _h2Pattern = RegExp(r'^##\s+(.+?)\s*#*\s*$');
final _fenceOpenPattern = RegExp(r'^ {0,3}(`{3,}|~{3,})');
final _fenceClosePattern = RegExp(r'^ {0,3}(`{3,}|~{3,})[ \t]*$');
final _metadataPattern = RegExp(
  r'^\s*<!--\s*buzz-board-card\s+(\{.*\})\s*-->\s*$',
);
final _cardIdPattern = RegExp(r'^[a-zA-Z0-9][a-zA-Z0-9._:-]{0,127}$');
final _eventIdPattern = RegExp(r'^[0-9a-f]{64}$');

class _SourceSection {
  final String title;
  final List<String> bodyLines;

  _SourceSection(this.title) : bodyLines = [];
}

String _derivedCardId(String title, int index) {
  final slug = title
      .toLowerCase()
      .replaceAll(RegExp(r'[^a-z0-9]+'), '-')
      .replaceAll(RegExp(r'^-|-$'), '');
  return '${slug.isEmpty ? 'card' : slug}-${index + 1}';
}

CanvasBoardCardType _inferCardType(String title) {
  final normalized = title.toLowerCase();
  if (RegExp(r'\b(agent|bot|berd)\b').hasMatch(normalized)) {
    return CanvasBoardCardType.agent;
  }
  if (RegExp(
    r'\b(person|people|member|steward|contributor)\b',
  ).hasMatch(normalized)) {
    return CanvasBoardCardType.person;
  }
  if (RegExp(r'\b(decision|decide|approved|verdict)\b').hasMatch(normalized)) {
    return CanvasBoardCardType.decision;
  }
  if (RegExp(
    r'\b(conversation|discussion|thread|work room)\b',
  ).hasMatch(normalized)) {
    return CanvasBoardCardType.conversation;
  }
  if (RegExp(
    r'\b(project|initiative|program|campaign)\b',
  ).hasMatch(normalized)) {
    return CanvasBoardCardType.project;
  }
  if (RegExp(
    r'\b(finished|made|shipped|artifact|showcase)\b',
  ).hasMatch(normalized)) {
    return CanvasBoardCardType.artifact;
  }
  if (RegExp(
    r'\b(task|todo|to-do|help|join|next|action|now|today|week|current|active)\b',
  ).hasMatch(normalized)) {
    return CanvasBoardCardType.task;
  }
  return CanvasBoardCardType.note;
}

CanvasBoardCardStatus _inferCardStatus(String title) {
  final normalized = title.toLowerCase();
  if (RegExp(
    r'\b(finished|made|shipped|artifact|showcase)\b',
  ).hasMatch(normalized)) {
    return CanvasBoardCardStatus.done;
  }
  if (RegExp(
    r'\b(now|today|week|current|happening|active)\b',
  ).hasMatch(normalized)) {
    return CanvasBoardCardStatus.doing;
  }
  return CanvasBoardCardStatus.backlog;
}

T? _enumByName<T extends Enum>(Iterable<T> values, Object? name) {
  if (name is! String) return null;
  for (final value in values) {
    if (value.name == name) return value;
  }
  return null;
}

Map<String, Object?>? _metadataFromFirstMeaningfulLine(List<String> lines) {
  final index = lines.indexWhere((line) => line.trim().isNotEmpty);
  if (index == -1) return null;
  final match = _metadataPattern.firstMatch(lines[index]);
  if (match == null) return null;
  try {
    final decoded = jsonDecode(match.group(1)!);
    if (decoded is! Map<String, dynamic>) return null;
    lines.removeAt(index);
    return decoded;
  } catch (_) {
    return null;
  }
}

String? _validString(Object? value, RegExp pattern) {
  if (value is! String) return null;
  final normalized = value.trim();
  return pattern.hasMatch(normalized) ? normalized : null;
}

/// Parses the same Markdown-backed card contract used by desktop Magic Board.
CanvasBoard parseCanvasBoard(String content) {
  final normalized = content.replaceAll(RegExp(r'\r\n?'), '\n');
  final introductionLines = <String>[];
  final sections = <_SourceSection>[];
  String? title;
  _SourceSection? activeSection;
  String? fenceCharacter;
  var fenceLength = 0;

  for (final line in normalized.split('\n')) {
    final wasInsideFence = fenceCharacter != null;
    if (fenceCharacter != null) {
      final closing = _fenceClosePattern.firstMatch(line)?.group(1);
      if (closing != null &&
          closing.startsWith(fenceCharacter) &&
          closing.length >= fenceLength) {
        fenceCharacter = null;
        fenceLength = 0;
      }
    } else {
      final opening = _fenceOpenPattern.firstMatch(line)?.group(1);
      if (opening != null) {
        fenceCharacter = opening[0];
        fenceLength = opening.length;
      }
    }
    final isFenceBoundary = wasInsideFence || fenceCharacter != null;

    if (!isFenceBoundary && activeSection == null) {
      final match = _h1Pattern.firstMatch(line);
      if (match != null && title == null) {
        title = match.group(1)!.trim();
        continue;
      }
    }
    if (!isFenceBoundary) {
      final match = _h2Pattern.firstMatch(line);
      if (match != null) {
        activeSection = _SourceSection(match.group(1)!.trim());
        sections.add(activeSection);
        continue;
      }
    }
    if (activeSection == null) {
      introductionLines.add(line);
    } else {
      activeSection.bodyLines.add(line);
    }
  }

  final introduction = introductionLines.join('\n').trim();
  final cards = <CanvasBoardCard>[
    for (final (index, section) in sections.indexed)
      (() {
        final bodyLines = [...section.bodyLines];
        final metadata = _metadataFromFirstMeaningfulLine(bodyLines);
        final id = _validString(metadata?['id'], _cardIdPattern);
        final rawThreadId = metadata?['thread'];
        final threadId = _validString(
          rawThreadId is String ? rawThreadId.toLowerCase() : null,
          _eventIdPattern,
        );
        final author = metadata?['author'];
        return CanvasBoardCard(
          id: id ?? _derivedCardId(section.title, index),
          title: section.title,
          body: bodyLines.join('\n').trim(),
          type:
              _enumByName(CanvasBoardCardType.values, metadata?['type']) ??
              _inferCardType(section.title),
          status:
              _enumByName(CanvasBoardCardStatus.values, metadata?['status']) ??
              _inferCardStatus(section.title),
          threadId: threadId,
          authorPubkey: author is String && author.trim().isNotEmpty
              ? author.trim()
              : null,
        );
      })(),
  ];

  if (cards.isEmpty && introduction.isNotEmpty) {
    cards.add(
      CanvasBoardCard(
        id: 'overview-1',
        title: 'Overview',
        body: introduction,
        type: CanvasBoardCardType.note,
        status: CanvasBoardCardStatus.backlog,
      ),
    );
  }

  return CanvasBoard(
    title: title,
    introduction: sections.isEmpty ? '' : introduction,
    cards: cards,
  );
}
