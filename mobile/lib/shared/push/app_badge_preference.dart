import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../theme/theme_provider.dart';

/// Whether the app icon badge counts only mentions and direct messages,
/// leaving ordinary channel unreads off it. Off by default.
final appBadgeMentionsOnlyProvider =
    NotifierProvider<AppBadgeMentionsOnly, bool>(AppBadgeMentionsOnly.new);

class AppBadgeMentionsOnly extends Notifier<bool> {
  static const _key = 'buzz-app-badge-mentions-only.v1';

  @override
  bool build() => ref.read(savedPrefsProvider).getBool(_key) ?? false;

  Future<void> set(bool mentionsOnly) async {
    state = mentionsOnly;
    await ref.read(savedPrefsProvider).setBool(_key, mentionsOnly);
  }
}
