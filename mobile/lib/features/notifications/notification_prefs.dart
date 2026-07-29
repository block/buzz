import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../shared/theme/theme_provider.dart';

/// SharedPreferences keys for notification preferences.
const _globalKey = 'buzz_notifications_global';
const _quietHoursEnabledKey = 'buzz_quiet_hours_enabled';
const _quietHoursStartKey = 'buzz_quiet_hours_start';
const _quietHoursEndKey = 'buzz_quiet_hours_end';

/// User-facing notification preferences, persisted locally via
/// SharedPreferences.
///
/// These control *whether* the device should show a push notification for
/// incoming messages. The relay-side push lease and channel mute logic are
/// independent — these prefs are the client-side gate that sits between
/// "relay says a push is warranted" and "device shows it".
class NotificationPrefs {
  /// Master switch. When `false`, no push notifications are shown regardless
  /// of other settings.
  final bool globalEnabled;

  /// Whether the quiet-hours schedule is active.
  final bool quietHoursEnabled;

  /// Start of the quiet window (inclusive). Notifications are suppressed while
  /// the current local time falls inside `[start, end)`. `null` means the user
  /// hasn't picked a time yet (defaults to 22:00 when the feature is first
  /// enabled).
  final TimeOfDay? quietHoursStart;

  /// End of the quiet window (exclusive). `null` defaults to 08:00.
  final TimeOfDay? quietHoursEnd;

  const NotificationPrefs({
    this.globalEnabled = true,
    this.quietHoursEnabled = false,
    this.quietHoursStart,
    this.quietHoursEnd,
  });

  /// Default quiet-hours window: 22:00 → 08:00.
  static const defaultStart = TimeOfDay(hour: 22, minute: 0);
  static const defaultEnd = TimeOfDay(hour: 8, minute: 0);

  /// Effective start/end, falling back to defaults when unset.
  TimeOfDay get effectiveStart => quietHoursStart ?? defaultStart;
  TimeOfDay get effectiveEnd => quietHoursEnd ?? defaultEnd;

  /// Whether the given [time] falls inside the quiet window.
  ///
  /// Handles overnight wrap-around: a window of 22:00→08:00 covers both
  /// late-evening and early-morning hours.
  bool isQuietHour(TimeOfDay time) {
    if (!quietHoursEnabled) return false;
    final now = time.hour * 60 + time.minute;
    final start = effectiveStart.hour * 60 + effectiveStart.minute;
    final end = effectiveEnd.hour * 60 + effectiveEnd.minute;
    if (start == end) return false;
    if (start < end) {
      return now >= start && now < end;
    }
    // Overnight: wraps past midnight.
    return now >= start || now < end;
  }

  /// Whether notifications should be *suppressed* right now, given the current
  /// local time [now].
  bool shouldSuppressAt(DateTime now) {
    if (!globalEnabled) return true;
    return isQuietHour(TimeOfDay.fromDateTime(now));
  }

  NotificationPrefs copyWith({
    bool? globalEnabled,
    bool? quietHoursEnabled,
    TimeOfDay? quietHoursStart,
    TimeOfDay? quietHoursEnd,
  }) {
    return NotificationPrefs(
      globalEnabled: globalEnabled ?? this.globalEnabled,
      quietHoursEnabled: quietHoursEnabled ?? this.quietHoursEnabled,
      quietHoursStart: quietHoursStart ?? this.quietHoursStart,
      quietHoursEnd: quietHoursEnd ?? this.quietHoursEnd,
    );
  }

  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      other is NotificationPrefs &&
          runtimeType == other.runtimeType &&
          globalEnabled == other.globalEnabled &&
          quietHoursEnabled == other.quietHoursEnabled &&
          _timeEquals(quietHoursStart, other.quietHoursStart) &&
          _timeEquals(quietHoursEnd, other.quietHoursEnd);

  @override
  int get hashCode => Object.hash(
    globalEnabled,
    quietHoursEnabled,
    quietHoursStart?.hour,
    quietHoursStart?.minute,
    quietHoursEnd?.hour,
    quietHoursEnd?.minute,
  );
}

bool _timeEquals(TimeOfDay? a, TimeOfDay? b) {
  if (a == null && b == null) return true;
  if (a == null || b == null) return false;
  return a.hour == b.hour && a.minute == b.minute;
}

/// Encodes/decodes [TimeOfDay] as "HH:MM" for SharedPreferences storage.
String _encodeTime(TimeOfDay t) =>
    '${t.hour.toString().padLeft(2, '0')}:${t.minute.toString().padLeft(2, '0')}';

TimeOfDay _decodeTime(String s) {
  final parts = s.split(':');
  return TimeOfDay(hour: int.parse(parts[0]), minute: int.parse(parts[1]));
}

/// Riverpod notifier that loads and persists [NotificationPrefs].
class NotificationPrefsNotifier extends Notifier<NotificationPrefs> {
  late final SharedPreferences _prefs;

  @override
  NotificationPrefs build() {
    _prefs = ref.read(savedPrefsProvider);
    return _load();
  }

  NotificationPrefs _load() {
    return NotificationPrefs(
      globalEnabled: _prefs.getBool(_globalKey) ?? true,
      quietHoursEnabled: _prefs.getBool(_quietHoursEnabledKey) ?? false,
      quietHoursStart: _prefs.getString(_quietHoursStartKey)?.let(_decodeTime),
      quietHoursEnd: _prefs.getString(_quietHoursEndKey)?.let(_decodeTime),
    );
  }

  void setGlobalEnabled(bool enabled) {
    _prefs.setBool(_globalKey, enabled);
    state = state.copyWith(globalEnabled: enabled);
  }

  void setQuietHoursEnabled(bool enabled) {
    _prefs.setBool(_quietHoursEnabledKey, enabled);
    state = state.copyWith(quietHoursEnabled: enabled);
  }

  void setQuietHoursStart(TimeOfDay time) {
    _prefs.setString(_quietHoursStartKey, _encodeTime(time));
    state = state.copyWith(quietHoursStart: time);
  }

  void setQuietHoursEnd(TimeOfDay time) {
    _prefs.setString(_quietHoursEndKey, _encodeTime(time));
    state = state.copyWith(quietHoursEnd: time);
  }
}

/// Provider for the current [NotificationPrefs].
final notificationPrefsProvider =
    NotifierProvider<NotificationPrefsNotifier, NotificationPrefs>(
      NotificationPrefsNotifier.new,
    );

/// Small extension to mimic Kotlin's `let` for nullable chaining.
extension _NullableLet<T> on T? {
  R? let<R>(R Function(T) fn) {
    final self = this;
    return self == null ? null : fn(self);
  }
}
