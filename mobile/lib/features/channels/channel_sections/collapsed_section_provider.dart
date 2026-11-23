import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../../shared/relay/relay.dart';

final collapsedSectionKeyProvider = Provider<String?>((ref) {
  final pubkey = ref.watch(myPubkeyProvider);
  if (pubkey == null) return null;
  final relay = ref.watch(relayConfigProvider.select((value) => value.baseUrl));
  return 'collapsed-sections:$relay:$pubkey';
});

final collapsedSectionsProvider =
    AsyncNotifierProvider.family<CollapsedSections, Set<String>, String?>(
      CollapsedSections.new,
    );

class CollapsedSections extends AsyncNotifier<Set<String>> {
  CollapsedSections(this.key);
  final String? key;
  late SharedPreferences _prefs;

  @override
  Future<Set<String>> build() async {
    final storageKey = key;
    if (storageKey == null) return {};
    _prefs = await SharedPreferences.getInstance();
    return {...?_prefs.getStringList(storageKey)};
  }

  Future<void> toggle(String section) async {
    if (!state.hasValue) await future;
    if (!ref.mounted) return;
    final next = {...state.requireValue};
    if (!next.remove(section)) next.add(section);
    state = AsyncData(next);
    final storageKey = key;
    if (storageKey != null &&
        !await _prefs.setStringList(storageKey, next.toList())) {
      throw StateError('Could not save collapsed sections');
    }
  }
}
