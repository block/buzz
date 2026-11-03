import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../shared/me/me_sync.dart';
import '../../shared/theme/theme_provider.dart';

/// Local onboarding belongs to one account, community, and conversation.
String bestieWelcomeKey(MeSync scope, String channelId) =>
    'buzz-bestie-welcome.v1:${Uri.encodeComponent(scope.community)}:'
    '${scope.pubkey}:$channelId';

/// Reads the local welcome independently of the currently selected Bestie.
final bestieWelcomeProvider = Provider.autoDispose.family<String?, String>((
  ref,
  channelId,
) {
  final scope = ref.watch(meSyncProvider);
  if (scope.config.nsec == null) return null;
  return ref
      .watch(savedPrefsProvider)
      .getString(bestieWelcomeKey(scope, channelId));
});

/// Retains the onboarding bubble without publishing a conversation event.
Future<void> saveBestieWelcome(
  SharedPreferences prefs,
  MeSync scope,
  String channelId,
  String agent,
) async {
  if (scope.disposed || scope.config.nsec == null) {
    throw StateError('Your account or community changed. Open the chat again.');
  }
  try {
    if (!await prefs.setString(bestieWelcomeKey(scope, channelId), agent)) {
      throw StateError('Could not save the welcome on this device.');
    }
  } catch (_) {
    // SharedPreferences can update its memory cache even when disk writing fails.
    await prefs.reload();
    rethrow;
  }
  if (scope.disposed || scope.config.nsec == null) {
    throw StateError('Your account or community changed. Open the chat again.');
  }
}
