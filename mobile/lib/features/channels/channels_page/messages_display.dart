import 'dart:convert';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../../../shared/theme/theme_provider.dart';
import '../channel_sections/section_reorder_sheet.dart';
import '../channel_filter/channel_filter_store.dart';
import '../channel_sort/channel_sort_storage.dart';

class MessagesDisplay {
  const MessagesDisplay({this.mode, this.count, this.days, this.sort});
  final ChannelSortMode? sort;
  final ChannelFilterMode? mode;
  final int? count;
  final int? days;

  ChannelFilterMode modeFor(ChannelFilterStore? store, String group) =>
      mode ?? store?.modeFor(group) ?? ChannelFilterMode.all;

  int? limitFor(ChannelFilterStore? store, String group) {
    final n = count ?? store?.countFor(group) ?? sectionCounts.first;
    return modeFor(store, group) == ChannelFilterMode.active || n == 0
        ? null
        : n;
  }

  int windowFor(ChannelFilterStore? store, String group) => days == null
      ? store?.windowFor(group) ?? defaultActiveWindowSeconds
      : days! * 86400;
}

class _DisplayPreferences extends Notifier<MessagesDisplay> {
  @override
  MessagesDisplay build() {
    final scope = ref.watch(mobileSectionOrderKeyProvider);
    if (scope == null) return const MessagesDisplay();
    final raw = ref
        .watch(savedPrefsProvider)
        .getString('messages-display.v1:$scope');
    if (raw == null) return const MessagesDisplay();
    try {
      final record = jsonDecode(raw);
      if (record is! Map) return const MessagesDisplay();
      return MessagesDisplay(
        sort: ChannelSortMode.fromWire(record['sort']),
        mode: ChannelFilterMode.values
            .where((m) => m.name == record['mode'])
            .firstOrNull,
        count: record['count'] is int && sectionCounts.contains(record['count'])
            ? record['count'] as int
            : null,
        days:
            record['days'] is int &&
                validActiveWindow((record['days'] as int) * 86400)
            ? record['days'] as int
            : null,
      );
    } on FormatException {
      return const MessagesDisplay();
    }
  }

  Future<void> update(MessagesDisplay display) async {
    final scope = ref.read(mobileSectionOrderKeyProvider);
    if (scope != null) {
      final saved = await ref
          .read(savedPrefsProvider)
          .setString(
            'messages-display.v1:$scope',
            jsonEncode({
              'sort': display.sort?.wireValue,
              'mode': display.mode?.name,
              'count': display.count,
              'days': display.days,
            }),
          );
      if (!saved) throw StateError('Could not save display options');
      if (!ref.mounted || ref.read(mobileSectionOrderKeyProvider) != scope) {
        return;
      }
    }
    state = display;
  }
}

final messagesDisplayProvider =
    NotifierProvider<_DisplayPreferences, MessagesDisplay>(
      _DisplayPreferences.new,
    );
