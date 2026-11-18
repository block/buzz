import 'dart:convert';
import 'me_groups.dart';

/// Concurrent edits require an explicit user decision.
class MeRecoveryConflict implements Exception {
  @override
  String toString() =>
      'Solo has conflicting edits on another device. Your pending change is still saved on this phone.';
}

/// Reapply only local edits to a newer desktop record. Concurrent edits to the
/// same field are surfaced, never silently overwritten.
MeGroups rebaseMeGroups(MeGroups base, MeGroups local, MeGroups remote) {
  bool same(Object? a, Object? b) => jsonEncode(a) == jsonEncode(b);
  Map<String, T> merge<T>(
    Map<String, T> before,
    Map<String, T> after,
    Map<String, T> current,
  ) {
    final result = {...current};
    for (final id in {...before.keys, ...after.keys}) {
      if (same(before[id], after[id])) continue;
      if (!same(current[id], before[id]) && !same(current[id], after[id])) {
        throw MeRecoveryConflict();
      }
      if (after.containsKey(id)) {
        result[id] = after[id] as T;
      } else {
        result.remove(id);
      }
    }
    return result;
  }

  Map<String, Map<String, dynamic>> groups(MeGroups value) => {
    for (final group in value.groups) group['id'] as String: group,
  };
  Map<String, bool> channels(MeGroups value) => {
    for (final id in value.channels) id: true,
  };
  return MeGroups(
    groups: merge(groups(base), groups(local), groups(remote)).values.toList(),
    assignments: merge(base.assignments, local.assignments, remote.assignments),
    channels: merge(
      channels(base),
      channels(local),
      channels(remote),
    ).keys.toList(),
  );
}
