import '../../shared/agents/is_quiet_agent_message.dart';
import 'date_formatters.dart';
import 'timeline_message.dart';

/// Consecutive agent-only messages that the timeline shows as one expandable
/// activity preview. Every message of a run maps to the same instance.
class ActivityRun {
  final List<TimelineMessage> rows;

  ActivityRun(this.rows);

  TimelineMessage get first => rows.first;
  TimelineMessage get latest => rows.last;

  /// Distinct authors, in order of first appearance.
  List<String> get participants =>
      {for (final row in rows) row.pubkey.toLowerCase()}.toList();
}

/// Each agent-only message in [rows] mapped to its run, matching desktop's
/// `activityRuns`: only consecutive agent-only messages fold together, and a
/// run splits at a new conversation, request or local day. Human messages,
/// public agent replies and system notices stay outside runs.
///
/// [rows] is one displayed level of the conversation, oldest first. [loaded]
/// is everything loaded, so a signed request claim can name a message shown
/// at another level. [rootId] is the thread being shown, if any. [agents]
/// adds known agent pubkeys to the authors seen posting agent-only messages.
Map<String, ActivityRun> activityRuns(
  List<TimelineMessage> rows, {
  Iterable<TimelineMessage> loaded = const [],
  String? rootId,
  Set<String> agents = const {},
}) {
  bool quiet(TimelineMessage row) =>
      !row.isSystem && isQuietAgentMessage(row.tags);
  final everything = [...rows, ...loaded];
  final agentAuthors = {
    for (final agent in agents) agent.toLowerCase(),
    for (final row in everything)
      if (quiet(row)) row.pubkey.toLowerCase(),
  };
  bool isAgent(TimelineMessage row) =>
      agentAuthors.contains(row.pubkey.toLowerCase());
  bool isRequest(TimelineMessage row) => !row.isSystem && !isAgent(row);
  final humanRequests = {
    for (final row in everything)
      if (isRequest(row)) row.id: row,
  };

  // Each agent row's request: a signed claim naming a loaded human request in
  // the same conversation, else its reply parent's, else the latest request
  // before it.
  final requests = <String, String>{};
  var request = rootId;
  for (final row in rows) {
    if (isRequest(row)) request = row.id;
    String? rowRequest = request;
    if (isAgent(row)) {
      final claimed = humanRequests[_requestClaim(row)];
      final claim =
          claimed != null &&
              (claimed.rootId == row.rootId || claimed.id == row.rootId)
          ? claimed.id
          : null;
      final parent = row.parentId;
      rowRequest =
          claim ??
          (parent == null ? null : requests[parent] ?? parent) ??
          request;
    }
    if (rowRequest != null) requests[row.id] = rowRequest;
  }

  final runs = <String, ActivityRun>{};
  ActivityRun? run;
  TimelineMessage? previous;
  for (final row in rows) {
    if (!quiet(row)) {
      run = null;
    } else {
      if (run == null ||
          previous!.rootId != row.rootId ||
          requests[previous.id] != requests[row.id] ||
          !isSameDay(previous.createdAt, row.createdAt)) {
        run = ActivityRun([]);
      }
      run.rows.add(row);
      runs[row.id] = run;
    }
    previous = row;
  }
  return runs;
}

String? _requestClaim(TimelineMessage row) {
  for (final tag in row.tags) {
    if (tag.length >= 2 && tag[0] == 'buzz:request') return tag[1];
  }
  return null;
}
