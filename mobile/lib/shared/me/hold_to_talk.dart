import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme_provider.dart';

/// Device-local preference for starting new voice notes from Bestie.
final holdToTalkProvider = NotifierProvider<HoldToTalkPreference, bool>(
  HoldToTalkPreference.new,
);

class HoldToTalkPreference extends Notifier<bool> {
  static const storageKey = 'hold_to_talk_to_bestie';

  @override
  bool build() => ref.watch(savedPrefsProvider).getBool(storageKey) ?? true;

  Future<void> setEnabled(bool enabled) async {
    final saved = await ref
        .read(savedPrefsProvider)
        .setBool(storageKey, enabled);
    if (!saved) throw StateError('Could not save the voice setting.');
    if (ref.mounted) state = enabled;
  }
}
