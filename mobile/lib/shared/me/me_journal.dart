import 'package:shared_preferences/shared_preferences.dart';

/// Remove only the reviewed record and restore memory after failed disk removal.
Future<void> clearMeJournal(
  SharedPreferences storage,
  String key,
  String expected, {
  required void Function() checkScope,
}) async {
  await storage.reload();
  checkScope();
  final current = storage.getString(key);
  if (current == null) return;
  if (current != expected) {
    throw StateError('The pending action changed. Review it again.');
  }
  try {
    if (!await storage.remove(key)) {
      throw StateError('Local cleanup failed.');
    }
  } catch (_) {
    // SharedPreferences removes the memory entry before the disk result arrives.
    await storage.reload();
    throw StateError(
      'Couldn’t clear the pending action on this phone. Retry to finish.',
    );
  }
}
