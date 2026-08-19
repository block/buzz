import 'dart:convert';

import 'observer_models.dart';

const _buzzReadTools = <String>{
  'get_messages',
  'get_channel_history',
  'get_thread',
  'search',
  'get_feed',
  'get_reactions',
  'list_channels',
  'get_channel',
  'get_users',
  'get_presence',
  'list_channel_members',
  'list_dms',
  'get_canvas',
  'list_workflows',
  'get_workflow_runs',
  'get_event',
  'get_user_notes',
  'get_contact_list',
};

const _buzzWriteTools = <String>{
  'send_message',
  'send_diff_message',
  'edit_message',
  'delete_message',
  'add_reaction',
  'remove_reaction',
  'join_channel',
  'leave_channel',
  'update_channel',
  'set_channel_topic',
  'set_channel_purpose',
  'open_dm',
  'set_profile',
  'set_presence',
  'trigger_workflow',
  'approve_step',
  'create_channel',
  'archive_channel',
  'unarchive_channel',
  'add_channel_member',
  'remove_channel_member',
  'add_dm_member',
  'hide_dm',
  'set_canvas',
  'create_workflow',
  'update_workflow',
  'delete_workflow',
  'set_channel_add_policy',
  'vote_on_post',
  'publish_note',
  'set_contact_list',
};

final _buzzToolNames = <String>{..._buzzReadTools, ..._buzzWriteTools};

final _buzzToolNamesByLength = _buzzToolNames.toList()
  ..sort((a, b) => b.length.compareTo(a.length));

final _buzzToolTitleAliases = <(RegExp, String)>[
  (RegExp(r'\bsending message to channel\b'), 'send_message'),
  (RegExp(r'\bretrieving recent messages from channel\b'), 'get_messages'),
  (RegExp(r'\bgetting channel details\b'), 'get_channel'),
  (RegExp(r'\bgetting user information\b'), 'get_users'),
  (RegExp(r'\bsearching relay history\b'), 'search'),
  (RegExp(r'\bgetting thread\b'), 'get_thread'),
  (RegExp(r'\badding reaction\b'), 'add_reaction'),
  (RegExp(r'\bremoving reaction\b'), 'remove_reaction'),
];

Map<String, dynamic> _asRecord(dynamic value) {
  if (value is Map<String, dynamic>) return value;
  if (value is Map) return value.cast<String, dynamic>();
  return const {};
}

String? _asString(dynamic value) {
  return value is String ? value : null;
}

String _shorten(String value) {
  if (value.length > 14) {
    return '${value.substring(0, 8)}...${value.substring(value.length - 4)}';
  }
  return value;
}

String _titleCase(String value) {
  return value
      .replaceAll(RegExp(r'[_-]+'), ' ')
      .replaceAll(RegExp(r'\s+'), ' ')
      .trim()
      .replaceAllMapped(RegExp(r'\b\w'), (m) => m[0]!.toUpperCase());
}

String _normalizeToolNameText(String value) {
  return value
      .trim()
      .toLowerCase()
      .replaceAll(RegExp(r'[^a-z0-9_]+'), '_')
      .replaceAll(RegExp(r'_+'), '_')
      .replaceAll(RegExp(r'^_+|_+$'), '');
}

String? _findBuzzToolName(String value, bool includeShortNames) {
  final alias = _findBuzzToolAlias(value);
  if (alias != null) return alias;

  final normalized = _normalizeToolNameText(value);
  for (final name in _buzzToolNamesByLength) {
    if ((!includeShortNames && name.length < 8) || !normalized.contains(name)) {
      continue;
    }
    return name;
  }
  return null;
}

String? _findBuzzToolAlias(String value) {
  final normalizedPhrase = value
      .trim()
      .toLowerCase()
      .replaceAll(RegExp(r'[_-]+'), ' ')
      .replaceAll(RegExp(r'\s+'), ' ');
  for (final (pattern, name) in _buzzToolTitleAliases) {
    if (pattern.hasMatch(normalizedPhrase)) return name;
  }
  return null;
}

bool _isGenericToolTitle(String value) {
  final normalized = _normalizeToolNameText(value);
  return normalized.isEmpty ||
      normalized == 'tool' ||
      normalized == 'tool_call' ||
      normalized == 'mcp_tool_call' ||
      normalized == 'unknown' ||
      normalized == 'read' ||
      normalized == 'write' ||
      normalized == 'execute' ||
      normalized == 'completed';
}

String _normalizeToolName(String title) {
  final knownName = _findBuzzToolName(title, true);
  if (knownName != null) return knownName;

  final normalized = _normalizeToolNameText(
    title,
  ).replaceAll(RegExp(r'^buzz_'), '');
  return RegExp(r'[a-z][a-z0-9_]+').firstMatch(normalized)?[0] ?? normalized;
}

ToolStatus _normalizeToolStatus(String status) {
  final normalized = status.toLowerCase();
  if (normalized.contains('complete') ||
      normalized.contains('success') ||
      normalized == 'done') {
    return ToolStatus.completed;
  }
  if (normalized.contains('fail') || normalized.contains('error')) {
    return ToolStatus.failed;
  }
  if (normalized.contains('pending')) {
    return ToolStatus.pending;
  }
  return ToolStatus.executing;
}

String _extractContentText(dynamic value) {
  if (value is String) return value;
  if (value is List) return value.map(_extractBlockText).join('\n');
  return _extractBlockText(value);
}

String _extractBlockText(dynamic value) {
  if (value is String) return value;
  if (value is List) return value.map(_extractBlockText).join('\n');
  final record = _asRecord(value);
  final nestedContent = record['content'];
  final rawOutput = record['rawOutput'];
  final nestedText = nestedContent != null && nestedContent is! String
      ? _extractBlockText(nestedContent)
      : '';
  final rawOutputText = rawOutput == null
      ? ''
      : rawOutput is String
      ? rawOutput
      : _safeJsonEncode(rawOutput);
  final directText = _asString(record['text']) ?? _asString(record['content']);
  if (directText != null && directText.isNotEmpty) return directText;
  if (nestedText.isNotEmpty) return nestedText;
  if (rawOutputText.isNotEmpty) return rawOutputText;
  return '';
}

String _extractPromptText(Map<String, dynamic> payload) {
  final params = _asRecord(payload['params']);
  final prompt = params['prompt'];
  if (prompt is! List) return '';
  return (prompt).map(_extractBlockText).where((s) => s.isNotEmpty).join('\n');
}

({List<PromptSection> sections, String userText, String userTitle})
_parsePromptText(String text) {
  final sections = _parsePromptSections(text);
  if (sections.isEmpty) {
    return (
      sections: <PromptSection>[],
      userText: text.trim(),
      userTitle: 'Prompt',
    );
  }

  PromptSection? eventSection;
  for (final section in sections) {
    if (section.title.toLowerCase().startsWith('buzz event')) {
      eventSection = section;
      break;
    }
  }
  final eventContent = eventSection != null
      ? _extractEventContent(eventSection.body)
      : '';
  final eventKind = eventSection?.title.split(':').skip(1).join(':').trim();

  return (
    sections: sections,
    userText: eventContent,
    userTitle: eventKind != null && eventKind.isNotEmpty
        ? _titleCase(eventKind)
        : 'Buzz event',
  );
}

List<PromptSection> _parsePromptSections(String text) {
  final sections = <PromptSection>[];
  final headerPattern = RegExp(r'^\[([^\]]+)]\s*$');
  String? currentTitle;
  final currentBody = StringBuffer();
  final preamble = <String>[];

  for (final line in text.split(RegExp(r'\r?\n'))) {
    final header = headerPattern.firstMatch(line);
    if (header != null) {
      if (currentTitle != null) {
        sections.add(
          PromptSection(
            title: currentTitle,
            body: currentBody.toString().trim(),
          ),
        );
      } else {
        final pre = preamble.join('\n').trim();
        if (pre.isNotEmpty) {
          sections.add(PromptSection(title: 'Prompt', body: pre));
        }
      }
      currentTitle = header.group(1)!;
      currentBody.clear();
      continue;
    }

    if (currentTitle != null) {
      if (currentBody.isNotEmpty) currentBody.write('\n');
      currentBody.write(line);
    } else {
      preamble.add(line);
    }
  }

  if (currentTitle != null) {
    sections.add(
      PromptSection(title: currentTitle, body: currentBody.toString().trim()),
    );
  } else {
    final pre = preamble.join('\n').trim();
    if (pre.isNotEmpty) {
      sections.add(PromptSection(title: 'Prompt', body: pre));
    }
  }

  return sections;
}

String _extractEventContent(String body) {
  final match = RegExp(r'^Content:\s*(.*)$', multiLine: true).firstMatch(body);
  return match?.group(1)?.trim() ?? '';
}

Map<String, dynamic> _extractToolArgs(Map<String, dynamic> update) {
  final candidates = [
    update['args'],
    update['arguments'],
    update['input'],
    update['rawInput'],
  ];
  for (final candidate in candidates) {
    if (candidate is Map && candidate is! List) {
      return Map<String, dynamic>.from(candidate);
    }
  }
  return const {};
}

({String title, String toolName, String? buzzToolName}) _extractToolIdentity(
  Map<String, dynamic> update,
) {
  final candidates = _collectToolNameCandidates(update);
  String? knownName;
  for (final c in candidates) {
    knownName = _findBuzzToolName(c, true);
    if (knownName != null) break;
  }
  knownName ??= _findBuzzToolName(_safeJsonEncode(update), false);
  String? firstSpecific;
  for (final candidate in candidates) {
    if (!_isGenericToolTitle(candidate)) {
      firstSpecific = candidate;
      break;
    }
  }
  final title =
      _asString(update['title']) ?? knownName ?? firstSpecific ?? 'Tool call';
  return (
    title: title,
    toolName: knownName ?? _normalizeToolName(firstSpecific ?? title),
    buzzToolName: knownName,
  );
}

List<String> _collectToolNameCandidates(Map<String, dynamic> update) {
  final args = _extractToolArgs(update);
  final tool = _asRecord(update['tool']);
  final input = _asRecord(update['input']);
  final rawInput = _asRecord(update['rawInput']);
  final sources = <dynamic>[
    update['toolName'],
    update['tool_name'],
    update['name'],
    update['title'],
    update['kind'],
    tool['name'],
    tool['toolName'],
    args['toolName'],
    args['tool_name'],
    args['name'],
    args['method'],
    input['toolName'],
    input['tool_name'],
    input['name'],
    rawInput['toolName'],
    rawInput['tool_name'],
    rawInput['name'],
  ];
  return [
    for (final s in sources)
      if (s is String && s.isNotEmpty) s,
  ];
}

String _extractToolResult(Map<String, dynamic> update) {
  final contentText = _extractContentText(update['content']);
  if (contentText.isNotEmpty) return contentText;
  return _extractBlockText(update['rawOutput']);
}

String _describeTurnStarted(dynamic payload) {
  final record = _asRecord(payload);
  final ids = record['triggeringEventIds'];
  if (ids is List) {
    final stringIds = ids.whereType<String>().toList();
    if (stringIds.isNotEmpty) {
      return 'Triggered by ${stringIds.map(_shorten).join(', ')}.';
    }
  }
  return 'Heartbeat or internal turn.';
}

String _describeSessionResolved(dynamic payload) {
  final record = _asRecord(payload);
  final sessionId = _asString(record['sessionId']);
  final isNewSession = record['isNewSession'] == true;
  if (sessionId == null) return 'Using existing ACP session.';
  return '${isNewSession ? 'Created' : 'Using'} session ${_shorten(sessionId)}.';
}

String _safeJsonEncode(dynamic value) {
  try {
    return jsonEncode(value);
  } catch (_) {
    return value.toString();
  }
}

List<TranscriptItem> buildTranscript(List<ObserverFrame> events) {
  final items = <TranscriptItem>[];
  final itemsById = <String, TranscriptItem>{};

  // Maps a logical message ID to the actual key currently being appended to.
  final activeMessageKey = <String, String>{};
  final sealedKeys = <String>{};
  var continuationSeq = 0;

  void sealOpenMessages() {
    for (final currentKey in activeMessageKey.values) {
      sealedKeys.add(currentKey);
    }
  }

  void upsertMessage(
    String id,
    String role,
    String title,
    String text,
    String timestamp,
  ) {
    final currentKey = activeMessageKey[id];

    if (currentKey != null && !sealedKeys.contains(currentKey)) {
      final existing = itemsById[currentKey];
      if (existing is MessageItem) {
        existing.text += text;
        return;
      }
    }

    continuationSeq += 1;
    final newKey = currentKey != null ? '$id:c$continuationSeq' : id;
    final item = MessageItem(
      id: newKey,
      role: role,
      title: title,
      text: text,
      timestamp: timestamp,
    );
    items.add(item);
    itemsById[newKey] = item;
    activeMessageKey[id] = newKey;
  }

  void upsertTextItem(
    String id,
    String type,
    String title,
    String text,
    String timestamp,
  ) {
    final existing = itemsById[id];
    if (existing != null) {
      if (type == 'thought' && existing is ThoughtItem) {
        existing.text += text;
        return;
      }
      if (type == 'lifecycle' && existing is LifecycleItem) {
        existing.text += text;
        return;
      }
    }
    sealOpenMessages();
    final TranscriptItem item;
    if (type == 'thought') {
      item = ThoughtItem(
        id: id,
        title: title,
        text: text,
        timestamp: timestamp,
      );
    } else {
      item = LifecycleItem(
        id: id,
        title: title,
        text: text,
        timestamp: timestamp,
      );
    }
    items.add(item);
    itemsById[id] = item;
  }

  void upsertMetadata(
    String id,
    String title,
    List<PromptSection> sections,
    String timestamp,
  ) {
    final existing = itemsById[id];
    if (existing is MetadataItem) {
      existing.sections = sections;
      return;
    }
    sealOpenMessages();
    final item = MetadataItem(
      id: id,
      title: title,
      sections: sections,
      timestamp: timestamp,
    );
    items.add(item);
    itemsById[id] = item;
  }

  void upsertTool(
    String id,
    String title,
    String toolName,
    String? buzzToolName,
    ToolStatus status,
    Map<String, dynamic> args,
    String result,
    bool isError,
    String timestamp,
  ) {
    final existing = itemsById[id];
    final canonicalBuzzToolName =
        buzzToolName ?? _findBuzzToolName(toolName, true);
    if (existing is ToolItem) {
      if (!_isGenericToolTitle(title)) {
        existing.title = title;
      }
      if (canonicalBuzzToolName != null) {
        existing.buzzToolName = canonicalBuzzToolName;
        existing.toolName = canonicalBuzzToolName;
      } else if (existing.buzzToolName == null &&
          !_isGenericToolTitle(toolName)) {
        existing.toolName = toolName;
      }
      existing.status = status;
      existing.args = args.isNotEmpty ? args : existing.args;
      if (result.isNotEmpty) existing.result = result;
      existing.isError = isError || existing.isError;
      return;
    }
    sealOpenMessages();
    final item = ToolItem(
      id: id,
      title: title,
      toolName: canonicalBuzzToolName ?? toolName,
      buzzToolName: canonicalBuzzToolName,
      status: status,
      args: args,
      result: result,
      isError: isError,
      timestamp: timestamp,
    );
    items.add(item);
    itemsById[id] = item;
  }

  for (final event in events) {
    if (event.kind == 'turn_started') {
      upsertTextItem(
        'turn:${event.turnId ?? '${event.seq}'}',
        'lifecycle',
        'Turn started',
        _describeTurnStarted(event.payload),
        event.timestamp,
      );
      continue;
    }

    if (event.kind == 'session_resolved') {
      upsertTextItem(
        'session:${event.turnId ?? '${event.seq}'}',
        'lifecycle',
        'Session ready',
        _describeSessionResolved(event.payload),
        event.timestamp,
      );
      continue;
    }

    if (event.kind == 'acp_parse_error') {
      upsertTextItem(
        'parse-error:${event.seq}',
        'lifecycle',
        'Wire parse error',
        _extractBlockText(event.payload),
        event.timestamp,
      );
      continue;
    }

    if (event.kind != 'acp_read' && event.kind != 'acp_write') {
      continue;
    }

    final payload = _asRecord(event.payload);
    final method = _asString(payload['method']);

    if (event.kind == 'acp_write' && method == 'session/prompt') {
      final promptText = _extractPromptText(payload);
      if (promptText.isNotEmpty) {
        final parsedPrompt = _parsePromptText(promptText);
        if (parsedPrompt.userText.isNotEmpty) {
          upsertMessage(
            'prompt:${event.turnId ?? '${event.seq}'}',
            'user',
            parsedPrompt.userTitle,
            parsedPrompt.userText,
            event.timestamp,
          );
        }
        if (parsedPrompt.sections.isNotEmpty) {
          upsertMetadata(
            'prompt-context:${event.turnId ?? '${event.seq}'}',
            'Prompt context',
            parsedPrompt.sections,
            event.timestamp,
          );
        }
      }
      continue;
    }

    if (event.kind != 'acp_read' || method != 'session/update') {
      continue;
    }

    final params = _asRecord(payload['params']);
    final update = _asRecord(params['update']);
    final updateType = _asString(update['sessionUpdate']) ?? 'unknown';
    final turnKey = event.turnId ?? event.sessionId ?? 'unknown';
    final messageId = _asString(update['messageId']);

    if (updateType == 'agent_message_chunk') {
      upsertMessage(
        'assistant:${messageId ?? turnKey}',
        'assistant',
        'Assistant',
        _extractContentText(update['content']),
        event.timestamp,
      );
      continue;
    }

    if (updateType == 'user_message_chunk') {
      upsertMessage(
        'user:${messageId ?? turnKey}',
        'user',
        'User',
        _extractContentText(update['content']),
        event.timestamp,
      );
      continue;
    }

    if (updateType == 'agent_thought_chunk') {
      upsertTextItem(
        'thinking:${messageId ?? turnKey}',
        'thought',
        'Thinking',
        _extractContentText(update['content']),
        event.timestamp,
      );
      continue;
    }

    if (updateType == 'tool_call') {
      final toolId = _asString(update['toolCallId']) ?? 'tool:${event.seq}';
      final identity = _extractToolIdentity(update);
      upsertTool(
        'tool:$toolId',
        identity.title,
        identity.toolName,
        identity.buzzToolName,
        _normalizeToolStatus(_asString(update['status']) ?? 'executing'),
        _extractToolArgs(update),
        _extractToolResult(update),
        false,
        event.timestamp,
      );
      continue;
    }

    if (updateType == 'tool_call_update') {
      final toolId = _asString(update['toolCallId']) ?? 'tool:${event.seq}';
      final status = _normalizeToolStatus(
        _asString(update['status']) ?? 'completed',
      );
      final identity = _extractToolIdentity(update);
      upsertTool(
        'tool:$toolId',
        identity.title,
        identity.toolName,
        identity.buzzToolName,
        status,
        _extractToolArgs(update),
        _extractToolResult(update),
        status == ToolStatus.failed,
        event.timestamp,
      );
      continue;
    }

    if (updateType == 'plan') {
      final content = _extractContentText(update['content']);
      upsertTextItem(
        'plan:$turnKey',
        'thought',
        'Plan',
        content.isNotEmpty ? content : _safeJsonEncode(update),
        event.timestamp,
      );
    }
  }

  return items;
}
