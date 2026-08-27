/// Authenticated client for the relay's `/api/tasks` routes.
///
/// Follows `RelayCommunityInviteActions` (`features/invites/`
/// `invite_create_provider.dart`), the app's existing NIP-98 REST caller:
/// resolve `baseUrl`/`nsec` from [relayConfigProvider], sign each request with
/// [buildNip98AuthHeader], and read the relay's `{"error": …}` body for a
/// message worth showing.
///
/// The routes are host-derived and tenant-scoped — there is no community id in
/// any path. `bind_community` resolves the community from the `Host` header and
/// NIP-98 binds the signature to that same host, so the community is decided by
/// which relay `baseUrl` points at.
library;

import 'dart:convert';

import 'package:http/http.dart' as http;
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../relay/relay.dart';
import 'task.dart';

/// Default per-request timeout, matching the invite minting call.
const _taskRequestTimeout = Duration(seconds: 15);

/// A non-2xx response from `/api/tasks`, carrying the relay's own message.
///
/// [toString] returns the bare message so it can be shown to a user directly;
/// `Exception.toString()` would prefix it with `Exception: `.
class TaskApiException implements Exception {
  /// Wraps a relay error [message] observed with [statusCode].
  TaskApiException(this.statusCode, this.message);

  /// HTTP status the relay returned.
  final int statusCode;

  /// Relay-supplied message, or a synthesized `HTTP <code>` fallback.
  final String message;

  @override
  String toString() => message;
}

/// Reads and writes tasks over the relay's NIP-98-authenticated REST surface.
class TasksApi {
  /// Binds a client to one relay and signing identity.
  TasksApi({
    required http.Client httpClient,
    required String baseUrl,
    required String? nsec,
  }) : _httpClient = httpClient,
       _baseUrl = baseUrl,
       _nsec = nsec;

  final http.Client _httpClient;
  final String _baseUrl;
  final String? _nsec;

  /// Whether this client has a signing key. Without one every call would throw
  /// from [buildNip98AuthHeader], so callers gate their UI on this instead.
  bool get canSign => _nsec != null && _nsec.isNotEmpty;

  /// `POST /api/tasks` — opens a task and returns it.
  Future<Task> createTask({
    required String title,
    String? body,
    String? channelId,
    String? sourceRef,
    String? assignee,
    int? priority,
    DateTime? dueAt,
    String source = 'mobile',
  }) async {
    final payload = buildCreateTaskPayload(
      title: title,
      body: body,
      channelId: channelId,
      sourceRef: sourceRef,
      assignee: assignee,
      priority: priority,
      dueAt: dueAt,
      source: source,
    );
    final decoded = await _send('POST', _uri('/api/tasks'), payload);
    return Task.fromJson(_asObject(decoded));
  }

  /// `GET /api/tasks` — this community's tasks, newest-modified first.
  ///
  /// Channel-bound tasks the caller cannot see are filtered out by the relay
  /// rather than failing the page.
  Future<List<Task>> listTasks({
    TaskStatus? status,
    String? channelId,
    String? assignee,
    int? limit,
  }) async {
    final query = <String, String>{
      if (status != null) 'status': status.wireValue,
      'channel': ?channelId,
      'assignee': ?assignee,
      if (limit != null) 'limit': '$limit',
    };
    final decoded = await _send('GET', _uri('/api/tasks', query), null);
    final tasks = _asObject(decoded)['tasks'];
    if (tasks is! List) {
      throw const FormatException('relay returned a malformed task list');
    }
    return [
      for (final task in tasks)
        if (task is Map<String, dynamic>) Task.fromJson(task),
    ];
  }

  /// `GET /api/tasks/{id}` — one task plus its full event history.
  Future<TaskDetail> getTask(String taskId) async {
    final decoded = _asObject(
      await _send('GET', _uri('/api/tasks/$taskId'), null),
    );
    final task = decoded['task'];
    if (task is! Map<String, dynamic>) {
      throw const FormatException('relay returned a malformed task');
    }
    final events = decoded['events'];
    return TaskDetail(
      task: Task.fromJson(task),
      events: [
        if (events is List)
          for (final event in events)
            if (event is Map<String, dynamic>) TaskEvent.fromJson(event),
      ],
    );
  }

  /// `PATCH /api/tasks/{id}` — updates a task and returns it.
  ///
  /// Only the fields passed here are sent, because the relay treats an absent
  /// key as "leave alone" and an explicit `null` as "clear". Passing nothing is
  /// rejected with a 400, so callers must change at least one field.
  Future<Task> updateTask(
    String taskId, {
    TaskStatus? status,
    String? title,
    int? priority,
  }) async {
    final payload = <String, Object?>{
      if (status != null) 'status': status.wireValue,
      if (title != null) 'title': title.trim(),
      'priority': ?priority,
    };
    final decoded = await _send('PATCH', _uri('/api/tasks/$taskId'), payload);
    return Task.fromJson(_asObject(decoded));
  }

  /// `POST /api/tasks/{id}/events` — appends a comment or the task's summary.
  ///
  /// A second [TaskEventAction.summaryPersisted] for the same task comes back
  /// as a 400 (`already has a persisted summary`) from the relay's partial
  /// unique index, surfaced here as a [TaskApiException].
  Future<TaskEvent> appendTaskEvent(
    String taskId, {
    required TaskEventAction action,
    required String body,
  }) async {
    final payload = buildTaskEventPayload(action: action, body: body);
    final decoded = await _send(
      'POST',
      _uri('/api/tasks/$taskId/events'),
      payload,
    );
    return TaskEvent.fromJson(_asObject(decoded));
  }

  Uri _uri(String path, [Map<String, String>? query]) {
    final base = Uri.parse(_baseUrl).resolve(path);
    if (query == null || query.isEmpty) return base;
    return base.replace(queryParameters: query);
  }

  /// Signs, sends, and decodes one request.
  ///
  /// The NIP-98 `u` tag must carry the full URL including the query string:
  /// the relay rebuilds its expected URL from the path plus the raw query it
  /// received, so signing the bare path would fail verification on any
  /// filtered `GET`.
  Future<Object?> _send(String method, Uri url, Map<String, Object?>? payload) {
    final bodyBytes = payload == null
        ? const <int>[]
        : utf8.encode(jsonEncode(payload));
    final request = http.Request(method, url)
      ..headers['Authorization'] = buildNip98AuthHeader(
        method: method,
        url: url.toString(),
        bodyBytes: bodyBytes,
        nsec: _nsec,
      );
    if (payload != null) {
      request.headers['Content-Type'] = 'application/json';
      request.bodyBytes = bodyBytes;
    }
    return _httpClient
        .send(request)
        .then(http.Response.fromStream)
        .timeout(_taskRequestTimeout)
        .then(_decode);
  }

  Object? _decode(http.Response response) {
    final dynamic decoded;
    try {
      decoded = response.body.isEmpty ? null : jsonDecode(response.body);
    } on FormatException {
      throw TaskApiException(
        response.statusCode,
        'The relay returned an unreadable task response.',
      );
    }
    if (response.statusCode < 200 || response.statusCode >= 300) {
      final rawMessage = decoded is Map<String, dynamic>
          ? decoded['error']
          : null;
      throw TaskApiException(
        response.statusCode,
        rawMessage is String && rawMessage.trim().isNotEmpty
            ? rawMessage
            : 'HTTP ${response.statusCode}',
      );
    }
    return decoded;
  }

  Map<String, dynamic> _asObject(Object? decoded) {
    if (decoded is! Map<String, dynamic>) {
      throw const FormatException('relay returned a malformed task response');
    }
    return decoded;
  }
}

/// Supplies the HTTP client used for task requests.
final tasksHttpClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

/// Supplies a task client bound to the active community and signing identity.
final tasksApiProvider = Provider<TasksApi>((ref) {
  final config = ref.watch(relayConfigProvider);
  return TasksApi(
    httpClient: ref.watch(tasksHttpClientProvider),
    baseUrl: config.baseUrl,
    nsec: config.nsec,
  );
});
