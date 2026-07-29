import 'package:buzz/features/notifications/notification_prefs.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('NotificationPrefs defaults', () {
    test('global notifications enabled by default', () {
      const prefs = NotificationPrefs();
      expect(prefs.globalEnabled, isTrue);
    });

    test('quiet hours disabled by default', () {
      const prefs = NotificationPrefs();
      expect(prefs.quietHoursEnabled, isFalse);
    });

    test('effective times fall back to defaults when unset', () {
      const prefs = NotificationPrefs();
      expect(prefs.effectiveStart, NotificationPrefs.defaultStart);
      expect(prefs.effectiveEnd, NotificationPrefs.defaultEnd);
    });

    test('effective times use custom values when set', () {
      const prefs = NotificationPrefs(
        quietHoursStart: TimeOfDay(hour: 23, minute: 30),
        quietHoursEnd: TimeOfDay(hour: 7, minute: 15),
      );
      expect(prefs.effectiveStart, const TimeOfDay(hour: 23, minute: 30));
      expect(prefs.effectiveEnd, const TimeOfDay(hour: 7, minute: 15));
    });
  });

  group('NotificationPrefs.isQuietHour', () {
    // Overnight window: 22:00 → 08:00
    final overnight = const NotificationPrefs(
      quietHoursEnabled: true,
      quietHoursStart: TimeOfDay(hour: 22, minute: 0),
      quietHoursEnd: TimeOfDay(hour: 8, minute: 0),
    );

    test('returns false when quiet hours disabled', () {
      const prefs = NotificationPrefs(quietHoursEnabled: false);
      expect(prefs.isQuietHour(const TimeOfDay(hour: 23, minute: 0)), isFalse);
    });

    test('inside overnight window (late evening)', () {
      expect(
        overnight.isQuietHour(const TimeOfDay(hour: 23, minute: 30)),
        isTrue,
      );
      expect(
        overnight.isQuietHour(const TimeOfDay(hour: 22, minute: 0)),
        isTrue,
      );
    });

    test('inside overnight window (early morning)', () {
      expect(
        overnight.isQuietHour(const TimeOfDay(hour: 3, minute: 0)),
        isTrue,
      );
      expect(
        overnight.isQuietHour(const TimeOfDay(hour: 7, minute: 59)),
        isTrue,
      );
    });

    test('outside overnight window (daytime)', () {
      expect(
        overnight.isQuietHour(const TimeOfDay(hour: 8, minute: 0)),
        isFalse,
      );
      expect(
        overnight.isQuietHour(const TimeOfDay(hour: 12, minute: 0)),
        isFalse,
      );
      expect(
        overnight.isQuietHour(const TimeOfDay(hour: 21, minute: 59)),
        isFalse,
      );
    });

    // Daytime window: 09:00 → 17:00
    final daytime = const NotificationPrefs(
      quietHoursEnabled: true,
      quietHoursStart: TimeOfDay(hour: 9, minute: 0),
      quietHoursEnd: TimeOfDay(hour: 17, minute: 0),
    );

    test('inside daytime window', () {
      expect(daytime.isQuietHour(const TimeOfDay(hour: 12, minute: 0)), isTrue);
      expect(daytime.isQuietHour(const TimeOfDay(hour: 9, minute: 0)), isTrue);
      expect(
        daytime.isQuietHour(const TimeOfDay(hour: 16, minute: 59)),
        isTrue,
      );
    });

    test('outside daytime window', () {
      expect(
        daytime.isQuietHour(const TimeOfDay(hour: 8, minute: 59)),
        isFalse,
      );
      expect(
        daytime.isQuietHour(const TimeOfDay(hour: 17, minute: 0)),
        isFalse,
      );
      expect(
        daytime.isQuietHour(const TimeOfDay(hour: 23, minute: 0)),
        isFalse,
      );
    });

    test('returns false when start equals end (empty window)', () {
      const prefs = NotificationPrefs(
        quietHoursEnabled: true,
        quietHoursStart: TimeOfDay(hour: 12, minute: 0),
        quietHoursEnd: TimeOfDay(hour: 12, minute: 0),
      );
      expect(prefs.isQuietHour(const TimeOfDay(hour: 12, minute: 0)), isFalse);
    });
  });

  group('NotificationPrefs.shouldSuppressAt', () {
    test('suppresses all when global disabled', () {
      const prefs = NotificationPrefs(globalEnabled: false);
      expect(prefs.shouldSuppressAt(DateTime(2026, 1, 1, 12, 0)), isTrue);
    });

    test('suppresses during quiet hours', () {
      const prefs = NotificationPrefs(
        globalEnabled: true,
        quietHoursEnabled: true,
        quietHoursStart: TimeOfDay(hour: 22, minute: 0),
        quietHoursEnd: TimeOfDay(hour: 8, minute: 0),
      );
      expect(prefs.shouldSuppressAt(DateTime(2026, 1, 1, 23, 30)), isTrue);
    });

    test('allows outside quiet hours', () {
      const prefs = NotificationPrefs(
        globalEnabled: true,
        quietHoursEnabled: true,
        quietHoursStart: TimeOfDay(hour: 22, minute: 0),
        quietHoursEnd: TimeOfDay(hour: 8, minute: 0),
      );
      expect(prefs.shouldSuppressAt(DateTime(2026, 1, 1, 14, 0)), isFalse);
    });

    test('allows when quiet hours disabled', () {
      const prefs = NotificationPrefs(
        globalEnabled: true,
        quietHoursEnabled: false,
      );
      expect(prefs.shouldSuppressAt(DateTime(2026, 1, 1, 2, 0)), isFalse);
    });
  });

  group('NotificationPrefs.copyWith', () {
    test('copies with partial changes', () {
      const original = NotificationPrefs();
      final updated = original.copyWith(globalEnabled: false);
      expect(updated.globalEnabled, isFalse);
      expect(updated.quietHoursEnabled, original.quietHoursEnabled);
    });

    test('preserves unchanged fields', () {
      const original = NotificationPrefs(
        globalEnabled: true,
        quietHoursEnabled: true,
        quietHoursStart: TimeOfDay(hour: 23, minute: 0),
      );
      final updated = original.copyWith(
        quietHoursEnd: const TimeOfDay(hour: 7, minute: 0),
      );
      expect(updated.globalEnabled, isTrue);
      expect(updated.quietHoursEnabled, isTrue);
      expect(updated.quietHoursStart, const TimeOfDay(hour: 23, minute: 0));
      expect(updated.quietHoursEnd, const TimeOfDay(hour: 7, minute: 0));
    });
  });

  group('NotificationPrefs equality', () {
    test('equal when all fields match', () {
      const a = NotificationPrefs(
        globalEnabled: true,
        quietHoursEnabled: true,
        quietHoursStart: TimeOfDay(hour: 22, minute: 0),
        quietHoursEnd: TimeOfDay(hour: 8, minute: 0),
      );
      const b = NotificationPrefs(
        globalEnabled: true,
        quietHoursEnabled: true,
        quietHoursStart: TimeOfDay(hour: 22, minute: 0),
        quietHoursEnd: TimeOfDay(hour: 8, minute: 0),
      );
      expect(a, equals(b));
      expect(a.hashCode, b.hashCode);
    });

    test('not equal when quiet hours differ', () {
      const a = NotificationPrefs(quietHoursEnabled: true);
      const b = NotificationPrefs(quietHoursEnabled: false);
      expect(a == b, isFalse);
    });

    test('not equal when times differ', () {
      const a = NotificationPrefs(
        quietHoursStart: TimeOfDay(hour: 22, minute: 0),
      );
      const b = NotificationPrefs(
        quietHoursStart: TimeOfDay(hour: 23, minute: 0),
      );
      expect(a == b, isFalse);
    });
  });
}
