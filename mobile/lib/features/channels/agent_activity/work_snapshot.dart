import 'observer_models.dart';

/// Owner-encrypted host progress. Each heartbeat is a complete bounded snapshot,
/// so opening chat late or reconnecting does not require historical replay.
class WorkSnapshot {
  const WorkSnapshot({
    required this.turnId,
    this.active = true,
    required this.threadRootId,
    required this.startedAt,
    required this.updatedAt,
    required this.steps,
  });
  final String turnId;
  final bool active;
  final String? threadRootId;
  final int startedAt;
  final int updatedAt;
  final List<String> steps;

  static WorkSnapshot? fromFrame(ObserverFrame frame) {
    if (frame.kind != 'work_snapshot' || frame.turnId == null) return null;
    final payload = frame.payload;
    if (payload is! Map<String, dynamic>) return null;
    final root = payload['threadRootId'];
    final active = payload['active'];
    final start = payload['startedAt'];
    final update = payload['updatedAt'];
    final steps = payload['steps'];
    if (active is! bool ||
        (root != null && root is! String) ||
        start is! int ||
        update is! int ||
        steps is! List ||
        steps.length > 32 ||
        steps.any((s) => s is! String || s.length > 320)) {
      return null;
    }
    return WorkSnapshot(
      turnId: frame.turnId!,
      active: active,
      threadRootId: root as String?,
      startedAt: start,
      updatedAt: update,
      steps: List<String>.unmodifiable(steps),
    );
  }
}

/// Exact thread and freshness are required; a sibling or old run is not live.
WorkSnapshot? currentWorkSnapshot(
  List<WorkSnapshot> snapshots,
  Set<String?> roots,
  int now,
) {
  final turns = <String, WorkSnapshot>{};
  for (final item in snapshots) {
    final previous = turns[item.turnId];
    if (previous == null || item.updatedAt >= previous.updatedAt) {
      turns[item.turnId] = item;
    }
  }
  WorkSnapshot? latest;
  for (final item in turns.values) {
    if (!item.active ||
        !roots.contains(item.threadRootId) ||
        item.updatedAt < now - 15000 ||
        item.updatedAt > now + 60000) {
      continue;
    }
    if (latest == null ||
        item.startedAt > latest.startedAt ||
        (item.startedAt == latest.startedAt &&
            item.updatedAt > latest.updatedAt)) {
      latest = item;
    }
  }
  return latest;
}
