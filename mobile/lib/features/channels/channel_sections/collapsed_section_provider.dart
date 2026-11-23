import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/relay/relay.dart';
import '../../../shared/theme/theme_provider.dart';

final collapsedSectionKeyProvider = Provider<String?>((ref) {
  final pubkey = ref.watch(myPubkeyProvider);
  if (pubkey == null) return null;
  final relay = ref.watch(relayConfigProvider.select((value) => value.baseUrl));
  return 'collapsed-sections:$relay:$pubkey';
});

final collapsedSectionsProvider =
    NotifierProvider.family<CollapsedSections, Set<String>, String?>(
      CollapsedSections.new,
    );

class CollapsedSections extends Notifier<Set<String>> {
  CollapsedSections(this.key);
  final String? key;

  // Preferences are preloaded in main(), so the first sidebar frame already
  // hides saved-collapsed rows instead of mounting and then animating them.
  @override
  Set<String> build() {
    final storageKey = key;
    if (storageKey == null) return {};
    return {...?ref.read(savedPrefsProvider).getStringList(storageKey)};
  }

  Future<void> toggle(String section) async {
    final next = {...state};
    if (!next.remove(section)) next.add(section);
    state = next;
    final storageKey = key;
    if (storageKey != null &&
        !await ref
            .read(savedPrefsProvider)
            .setStringList(storageKey, next.toList())) {
      throw StateError('Could not save collapsed sections');
    }
  }
}
