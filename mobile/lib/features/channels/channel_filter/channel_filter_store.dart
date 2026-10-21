import 'dart:convert';
import 'dart:math';

const defaultActiveWindowSeconds = 30 * 86400;

enum ChannelFilterMode { all, unreads, active }

bool validActiveWindow(Object? value) =>
    value is int &&
    value >= 86400 &&
    value <= 365 * 86400 &&
    value % 86400 == 0;

/// Retains the entire encrypted record. Metadata is authoritative, including
/// null reset tombstones; legacy groups are only a compatibility projection.
class ChannelFilterStore {
  final Map<String, dynamic> _record;
  ChannelFilterStore._(this._record);
  factory ChannelFilterStore.empty() =>
      ChannelFilterStore._({'version': 1, 'groups': <String, dynamic>{}});
  factory ChannelFilterStore.parse(Map<String, dynamic> record) {
    if (record['version'] != 1 ||
        utf8.encode(jsonEncode(record)).length > 128 * 1024) {
      throw const FormatException('Unsupported channel filter');
    }
    final copy = jsonDecode(jsonEncode(record)) as Map<String, dynamic>;
    // Salvage individual v1 nodes from the authenticated relay head. Never use
    // the display cache for repair, and never reinterpret a future schema.
    Map<String, dynamic> salvage(
      Object? value,
      bool Function(Object?) valid, {
      int limit = 104,
    }) {
      if (value is! Map) return {};
      final result = <String, dynamic>{};
      for (final entry in value.entries) {
        if (_key(entry.key) && valid(entry.value)) {
          result[entry.key as String] = entry.value;
          if (result.length > limit) {
            throw const FormatException('Filter node budget exceeded');
          }
        }
      }
      return result;
    }

    copy['groups'] = salvage(copy['groups'], _mode);
    if (copy.containsKey('activeWindowSeconds')) {
      copy['activeWindowSeconds'] = salvage(
        copy['activeWindowSeconds'],
        validActiveWindow,
      );
    }
    if (copy['meta'] != null) {
      final raw = copy['meta'];
      if (raw is Map && raw['v'] != null && raw['v'] != 1) {
        throw const FormatException('Unsupported filter metadata');
      }
      final registers = salvage(
        raw is Map ? raw['g'] : null,
        (r) =>
            r is List &&
            r.length == 3 &&
            r[0] is int &&
            r[0] >= 0 &&
            r[0] <= 9007199254740991 &&
            r[1] is String &&
            RegExp(r'^[0-9a-f]{16}$').hasMatch(r[1]) &&
            (r[2] == null || r[2] == 'unreads' || r[2] == 'active'),
        limit: 2048,
      );
      // The companion admits only v/g inside metadata; top-level extensions
      // remain preserved above. Explicit saves publish this canonical repair.
      copy['meta'] = {'v': 1, 'g': registers};
      // Metadata remains authoritative: dropping an invalid register must not
      // resurrect its stale legacy projection (including an All tombstone).
      copy['groups'] = {
        for (final entry in registers.entries)
          if ((entry.value as List)[2] != null) entry.key: entry.value[2],
      };
      if ((copy['groups'] as Map).length > 104) {
        throw const FormatException('Filter group budget exceeded');
      }
    }
    return ChannelFilterStore._(copy);
  }
  static bool _key(Object? k) =>
      k is String && k.trim().isNotEmpty && k.length <= 264 && k != '__proto__';
  static bool _mode(Object? v) => v == 'all' || v == 'unreads' || v == 'active';
  ChannelFilterMode modeFor(String group) {
    final meta = _record['meta'];
    final value = meta == null
        ? (_record['groups'] as Map)[group]
        : ((meta['g'] as Map)[group] as List?)?[2];
    return switch (value) {
      'unreads' => ChannelFilterMode.unreads,
      'active' => ChannelFilterMode.active,
      _ => ChannelFilterMode.all,
    };
  }

  int windowFor(String group) =>
      (_record['activeWindowSeconds'] as Map?)?[group] as int? ??
      defaultActiveWindowSeconds;
  Map<String, dynamic> toJson() =>
      jsonDecode(jsonEncode(_record)) as Map<String, dynamic>;

  ChannelFilterStore edit(
    String group,
    ChannelFilterMode mode, {
    int? windowSeconds,
    bool removeWindow = false,
    int remoteCreatedAt = 0,
  }) {
    if (!_key(group) ||
        (windowSeconds != null &&
            (!validActiveWindow(windowSeconds) ||
                mode != ChannelFilterMode.active))) {
      throw const FormatException('Invalid filter choice');
    }
    final data = toJson();
    final meta =
        (data['meta'] as Map?)?.cast<String, dynamic>() ??
        {
          'v': 1,
          'g': <String, dynamic>{
            for (final entry in (data['groups'] as Map).entries)
              if (entry.value != 'all')
                entry.key as String: [
                  remoteCreatedAt * 1000,
                  '0000000000000000',
                  entry.value,
                ],
          },
        };
    final registers = (meta['g'] as Map).cast<String, dynamic>();
    var stamp = DateTime.now().millisecondsSinceEpoch;
    for (final r in registers.values) {
      stamp = max(stamp, ((r as List)[0] as int) + 1);
    }
    final random = Random.secure();
    final device = List.generate(
      8,
      (_) => random.nextInt(256).toRadixString(16).padLeft(2, '0'),
    ).join();
    registers[group] = [
      stamp,
      device,
      mode == ChannelFilterMode.all ? null : mode.name,
    ];
    data['meta'] = {...meta, 'g': registers};
    data['groups'] = {
      for (final e in registers.entries)
        if ((e.value as List)[2] != null) e.key: e.value[2],
    };
    final windows =
        (data['activeWindowSeconds'] as Map?)?.cast<String, dynamic>() ??
        <String, dynamic>{};
    if (removeWindow) {
      windows.remove(group);
    } else if (windowSeconds != null) {
      windows[group] = windowSeconds;
    }
    data['activeWindowSeconds'] = windows;
    return ChannelFilterStore.parse(data);
  }
}

bool channelMatchesFilter({
  required ChannelFilterMode mode,
  required bool unread,
  required int? activitySeconds,
  required bool activityKnown,
  required int nowSeconds,
  required int windowSeconds,
  bool selected = false,
}) =>
    selected ||
    switch (mode) {
      ChannelFilterMode.all => true,
      ChannelFilterMode.unreads => unread,
      ChannelFilterMode.active =>
        activitySeconds == null
            ? !activityKnown
            : activitySeconds + windowSeconds > nowSeconds,
    };
