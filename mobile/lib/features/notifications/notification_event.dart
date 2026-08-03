import 'package:flutter/foundation.dart';

enum NotificationCategory { priority, activity }

@immutable
class NotificationEvent {
  const NotificationEvent({
    required this.eventId,
    required this.id,
    required this.category,
    required this.title,
    required this.body,
    required this.route,
  });

  final String eventId;
  final int id;
  final NotificationCategory category;
  final String title;
  final String body;
  final String route;

  String get channel => category.name;
}

bool notificationCategoryEnabled({
  required NotificationCategory category,
  required bool alertsEnabled,
  required bool priorityEnabled,
  required bool activityEnabled,
}) {
  if (!alertsEnabled) return false;
  return switch (category) {
    NotificationCategory.priority => priorityEnabled,
    NotificationCategory.activity => activityEnabled,
  };
}
