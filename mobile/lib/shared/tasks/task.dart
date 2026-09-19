/// Task and task-event models plus the request-payload builders for the
/// relay's `/api/tasks` surface.
///
/// The wire format is deliberately asymmetric, so the encode and decode paths
/// below are not mirror images:
///
/// * **Outbound** `due_at` is an RFC 3339 string — `api::tasks` deserializes it
///   into a `chrono::DateTime<Utc>`.
/// * **Inbound** `due_at`, `created_at`, `updated_at`, `done_at` and
///   `archived_at` are Unix **seconds**, because `task_json` emits
///   `value.timestamp()`.
///
/// Keep this file free of Flutter and network imports: everything here is a
/// pure function or a value type so it can be unit-tested without a harness.
library;

import 'package:flutter/foundation.dart';

/// Longest title the relay accepts, counted the way it counts.
///
/// `api::tasks::validate_title` uses `chars().count()` and the table's
/// `CHECK (length(title) BETWEEN 1 AND 200)` counts characters too, so the
/// client-side guard must count Unicode scalar values — `String.runes`, not
/// `String.length` (UTF-16 code units) and not grapheme clusters.
const maxTaskTitleChars = 200;

/// Number of characters the relay will count in [title].
int taskTitleLength(String title) => title.runes.length;

/// Lifecycle state of a task, mirroring `buzz_core::task::TaskStatus`.
enum TaskStatus {
  /// Open and unstarted.
  todo('todo'),

  /// Being worked on.
  inProgress('in_progress'),

  /// Waiting on something else.
  blocked('blocked'),

  /// Finished.
  done('done'),

  /// Abandoned.
  cancelled('cancelled');

  const TaskStatus(this.wireValue);

  /// The string the relay reads and writes for this status.
  final String wireValue;

  /// Parses a relay status string.
  ///
  /// An unrecognised value degrades to [TaskStatus.todo] rather than throwing:
  /// a client built before a status was added must still render the rest of a
  /// task list during a rolling upgrade.
  static TaskStatus fromWire(String? raw) {
    for (final status in TaskStatus.values) {
      if (status.wireValue == raw) return status;
    }
    return TaskStatus.todo;
  }
}

/// The two task-event actions a client may post directly.
///
/// `api::tasks::append_task_event` rejects every other action, because
/// lifecycle entries (`created`, `status_changed`, `assigned`,
/// `title_changed`) are derived from the mutation that caused them — accepting
/// one from a caller would let it fabricate a history that never happened.
enum TaskEventAction {
  /// A free-text comment.
  commented('commented'),

  /// The task's single persisted summary. The relay's
  /// `idx_task_events_one_summary_per_task` partial unique index rejects a
  /// second one with a 400.
  summaryPersisted('summary_persisted');

  const TaskEventAction(this.wireValue);

  /// The string the relay reads for this action.
  final String wireValue;
}

/// Decodes a relay timestamp field (Unix seconds) into local time.
DateTime? _dateFromSeconds(Object? value) {
  if (value is! int) return null;
  return DateTime.fromMillisecondsSinceEpoch(value * 1000, isUtc: true);
}

String? _stringOrNull(Object? value) => value is String ? value : null;

/// A task as returned by `GET`/`POST`/`PATCH /api/tasks`.
@immutable
class Task {
  /// Creates a task record.
  const Task({
    required this.id,
    required this.title,
    required this.status,
    required this.priority,
    required this.createdAt,
    required this.updatedAt,
    this.channelId,
    this.createdBy,
    this.assignee,
    this.parentTaskId,
    this.body,
    this.source,
    this.sourceRef,
    this.dueAt,
    this.doneAt,
    this.archivedAt,
  });

  /// Parses one relay task object.
  factory Task.fromJson(Map<String, dynamic> json) {
    final id = _stringOrNull(json['id']);
    final title = _stringOrNull(json['title']);
    if (id == null || title == null) {
      throw const FormatException('relay returned a task without id or title');
    }
    return Task(
      id: id,
      title: title,
      status: TaskStatus.fromWire(_stringOrNull(json['status'])),
      priority: json['priority'] is int ? json['priority'] as int : 0,
      createdAt: _dateFromSeconds(json['created_at']) ?? DateTime.now().toUtc(),
      updatedAt: _dateFromSeconds(json['updated_at']) ?? DateTime.now().toUtc(),
      channelId: _stringOrNull(json['channel_id']),
      createdBy: _stringOrNull(json['created_by']),
      assignee: _stringOrNull(json['assignee']),
      parentTaskId: _stringOrNull(json['parent_task_id']),
      body: _stringOrNull(json['body']),
      source: _stringOrNull(json['source']),
      sourceRef: _stringOrNull(json['source_ref']),
      dueAt: _dateFromSeconds(json['due_at']),
      doneAt: _dateFromSeconds(json['done_at']),
      archivedAt: _dateFromSeconds(json['archived_at']),
    );
  }

  /// Task id, unique within its community.
  final String id;

  /// Single-line summary of the work.
  final String title;

  /// Lifecycle state.
  final TaskStatus status;

  /// Higher sorts first in the relay's list order.
  final int priority;

  /// When the task was opened.
  final DateTime createdAt;

  /// When the task last changed.
  final DateTime updatedAt;

  /// Channel this task is scoped to, or null for a community-wide task.
  final String? channelId;

  /// Hex pubkey of the author.
  final String? createdBy;

  /// Hex pubkey of the assignee.
  final String? assignee;

  /// Parent task id for a subtask.
  final String? parentTaskId;

  /// Optional Markdown detail.
  final String? body;

  /// Harness or client that opened the task.
  final String? source;

  /// Originating external reference — for a task opened from a thread, the
  /// event id of the message it came from.
  final String? sourceRef;

  /// When the work is due.
  final DateTime? dueAt;

  /// When the task was completed.
  final DateTime? doneAt;

  /// When the task was archived.
  final DateTime? archivedAt;
}

/// One entry in a task's history.
@immutable
class TaskEvent {
  /// Creates a task-event record.
  const TaskEvent({
    required this.id,
    required this.taskId,
    required this.action,
    required this.createdAt,
    this.actor,
    this.fromStatus,
    this.toStatus,
    this.body,
  });

  /// Parses one relay task-event object.
  factory TaskEvent.fromJson(Map<String, dynamic> json) {
    final taskId = _stringOrNull(json['task_id']);
    final action = _stringOrNull(json['action']);
    if (taskId == null || action == null) {
      throw const FormatException('relay returned a malformed task event');
    }
    return TaskEvent(
      id: json['id'] is int ? json['id'] as int : 0,
      taskId: taskId,
      action: action,
      createdAt: _dateFromSeconds(json['created_at']) ?? DateTime.now().toUtc(),
      actor: _stringOrNull(json['actor']),
      fromStatus: _stringOrNull(json['from_status']),
      toStatus: _stringOrNull(json['to_status']),
      body: _stringOrNull(json['body']),
    );
  }

  /// Monotonic event id.
  final int id;

  /// The task this entry belongs to.
  final String taskId;

  /// Raw action string.
  ///
  /// Kept as text rather than an enum because `task_events.action` is
  /// deliberately unconstrained `TEXT` — a new harness must be able to write a
  /// new action without a schema migration, and this client must not choke on
  /// one it has never seen.
  final String action;

  /// When the entry was written.
  final DateTime createdAt;

  /// Hex pubkey of whoever caused the entry.
  final String? actor;

  /// Status before a transition.
  final String? fromStatus;

  /// Status after a transition.
  final String? toStatus;

  /// Free text — a comment, or a persisted summary.
  final String? body;

  /// Whether this entry is the task's persisted summary.
  bool get isSummary => action == TaskEventAction.summaryPersisted.wireValue;
}

/// A task plus its full event history, as returned by `GET /api/tasks/{id}`.
@immutable
class TaskDetail {
  /// Pairs a task with its history.
  const TaskDetail({required this.task, required this.events});

  /// The task.
  final Task task;

  /// Its history, oldest first.
  final List<TaskEvent> events;

  /// The persisted summary entry, if one exists.
  ///
  /// At most one can exist per task — the relay enforces it with a partial
  /// unique index — so callers can use this to decide between "persist" and
  /// "already summarized".
  TaskEvent? get summary {
    for (final event in events) {
      if (event.isSummary) return event;
    }
    return null;
  }
}

/// Builds the JSON body for `POST /api/tasks`.
///
/// Null and blank optional fields are omitted rather than sent as `null`, so
/// the relay's own defaults apply.
///
/// Throws [ArgumentError] when [title] is blank or longer than
/// [maxTaskTitleChars]: the same rejection `validate_title` would return as a
/// 400, raised locally so the sheet can show it without a round trip.
Map<String, Object?> buildCreateTaskPayload({
  required String title,
  String? body,
  String? channelId,
  String? sourceRef,
  String? assignee,
  int? priority,
  DateTime? dueAt,
  String source = 'mobile',
}) {
  final trimmedTitle = title.trim();
  if (trimmedTitle.isEmpty) {
    throw ArgumentError.value(title, 'title', 'must not be empty');
  }
  if (taskTitleLength(trimmedTitle) > maxTaskTitleChars) {
    throw ArgumentError.value(
      title,
      'title',
      'must be at most $maxTaskTitleChars characters',
    );
  }
  final trimmedBody = body?.trim();
  return <String, Object?>{
    'title': trimmedTitle,
    if (trimmedBody != null && trimmedBody.isNotEmpty) 'body': trimmedBody,
    'channel_id': ?channelId,
    'source_ref': ?sourceRef,
    'assignee': ?assignee,
    'priority': ?priority,
    // RFC 3339 in UTC: the relay parses this into `DateTime<Utc>` and only
    // echoes Unix seconds back, so the outbound shape is not the inbound one.
    if (dueAt != null) 'due_at': dueAt.toUtc().toIso8601String(),
    'source': source,
  };
}

/// Resolves the `@handle` labels for a channel's agents.
///
/// Precedence matches the mention pipeline: the agent's own profile display
/// name, then the relay agent directory's, then the first 8 hex characters of
/// its pubkey. All three lookups are lowercase-keyed.
///
/// The result is sorted case-insensitively because [agentPubkeys] arrives as a
/// `Set` — iteration order there is not part of its contract, and an unsorted
/// result would make the composed body unstable between rebuilds.
List<String> resolveAgentHandles({
  required Iterable<String> agentPubkeys,
  required Map<String, String> profileNames,
  required Map<String, String> directoryNames,
}) {
  final handles = <String>[];
  for (final rawPubkey in agentPubkeys) {
    final pubkey = rawPubkey.toLowerCase();
    final profileName = profileNames[pubkey]?.trim();
    final directoryName = directoryNames[pubkey]?.trim();
    final handle = switch ((profileName, directoryName)) {
      (final String name, _) when name.isNotEmpty => name,
      (_, final String name) when name.isNotEmpty => name,
      _ => pubkey.length >= 8 ? pubkey.substring(0, 8) : pubkey,
    };
    if (handle.isNotEmpty && !handles.contains(handle)) handles.add(handle);
  }
  handles.sort((a, b) => a.toLowerCase().compareTo(b.toLowerCase()));
  return handles;
}

/// Combines an optional [body] with an `@`-mention line for [agentHandles].
///
/// Returns null when there is nothing to send, so the caller can omit `body`
/// from the payload entirely rather than posting an empty string.
///
/// The mention line leads because it is the addressing, not the detail: an
/// agent reading the task should see who it is for on the first line.
String? composeTaskBody({String? body, List<String> agentHandles = const []}) {
  final trimmedBody = body?.trim() ?? '';
  final mentions = [
    for (final handle in agentHandles)
      if (handle.trim().isNotEmpty) '@${handle.trim()}',
  ];
  if (mentions.isEmpty) return trimmedBody.isEmpty ? null : trimmedBody;
  final mentionLine = mentions.join(' ');
  return trimmedBody.isEmpty ? mentionLine : '$mentionLine\n\n$trimmedBody';
}

/// Builds the JSON body for `POST /api/tasks/{id}/events`.
///
/// Throws [ArgumentError] for a blank body, which the relay rejects with a 400.
Map<String, Object?> buildTaskEventPayload({
  required TaskEventAction action,
  required String body,
}) {
  final trimmed = body.trim();
  if (trimmed.isEmpty) {
    throw ArgumentError.value(body, 'body', 'must not be empty');
  }
  return <String, Object?>{'action': action.wireValue, 'body': trimmed};
}
