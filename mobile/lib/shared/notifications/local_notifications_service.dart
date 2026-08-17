import 'package:flutter/foundation.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';

import '../deeplink/deep_link.dart';

typedef InboxNotificationTapHandler = void Function(MessageDeepLink link);

const _androidChannelId = 'buzz_inbox';
const _androidChannelName = 'Inbox';

/// Thin wrapper around [FlutterLocalNotificationsPlugin] for inbox alerts.
class LocalNotificationsService {
  LocalNotificationsService._();

  static final LocalNotificationsService instance =
      LocalNotificationsService._();

  final FlutterLocalNotificationsPlugin _plugin =
      FlutterLocalNotificationsPlugin();
  InboxNotificationTapHandler? _onTap;
  bool _initialized = false;

  @visibleForTesting
  FlutterLocalNotificationsPlugin get plugin => _plugin;

  Future<void> initialize({required InboxNotificationTapHandler onTap}) async {
    if (_initialized) return;
    _onTap = onTap;

    const androidSettings = AndroidInitializationSettings(
      '@mipmap/ic_launcher',
    );
    const iosSettings = DarwinInitializationSettings(
      requestAlertPermission: false,
      requestBadgePermission: false,
      requestSoundPermission: false,
    );
    const settings = InitializationSettings(
      android: androidSettings,
      iOS: iosSettings,
    );

    await _plugin.initialize(
      settings: settings,
      onDidReceiveNotificationResponse: _handleNotificationResponse,
      onDidReceiveBackgroundNotificationResponse: _backgroundTapHandler,
    );

    final androidPlugin = _plugin
        .resolvePlatformSpecificImplementation<
          AndroidFlutterLocalNotificationsPlugin
        >();
    await androidPlugin?.createNotificationChannel(
      const AndroidNotificationChannel(
        _androidChannelId,
        _androidChannelName,
        description: 'Mentions, inbox items, and agent job results',
        importance: Importance.high,
      ),
    );

    _initialized = true;
    await _dispatchLaunchNotification();
  }

  Future<void> requestPermissionsIfNeeded() async {
    final iosPlugin = _plugin
        .resolvePlatformSpecificImplementation<
          IOSFlutterLocalNotificationsPlugin
        >();
    await iosPlugin?.requestPermissions(alert: true, badge: true, sound: true);

    final androidPlugin = _plugin
        .resolvePlatformSpecificImplementation<
          AndroidFlutterLocalNotificationsPlugin
        >();
    await androidPlugin?.requestNotificationsPermission();
  }

  Future<bool> showInboxNotification({
    required int notificationId,
    required String title,
    required String body,
    required String payload,
  }) async {
    if (!_initialized) return false;

    final details = NotificationDetails(
      android: AndroidNotificationDetails(
        _androidChannelId,
        _androidChannelName,
        channelDescription: 'Mentions, inbox items, and agent job results',
        importance: Importance.high,
        priority: Priority.high,
      ),
      iOS: const DarwinNotificationDetails(
        presentAlert: true,
        presentBadge: true,
        presentSound: true,
      ),
    );

    await _plugin.show(
      id: notificationId,
      title: title,
      body: body,
      notificationDetails: details,
      payload: payload,
    );
    return true;
  }

  Future<void> _dispatchLaunchNotification() async {
    final launchDetails = await _plugin.getNotificationAppLaunchDetails();
    if (launchDetails?.didNotificationLaunchApp != true) return;
    final payload = launchDetails?.notificationResponse?.payload;
    _dispatchPayload(payload);
  }

  void _handleNotificationResponse(NotificationResponse response) {
    _dispatchPayload(response.payload);
  }

  void _dispatchPayload(String? payload) {
    if (payload == null || payload.isEmpty) return;
    final uri = Uri.tryParse(payload);
    if (uri == null) return;
    final link = parseMessageDeepLink(uri);
    if (link == null) return;
    _onTap?.call(link);
  }

  @pragma('vm:entry-point')
  static void _backgroundTapHandler(NotificationResponse response) {
    // Foreground/background taps are handled by onDidReceiveNotificationResponse.
  }
}

int stableInboxNotificationId(String eventId) {
  return eventId.hashCode & 0x7fffffff;
}

String encodeInboxNotificationPayloadForTest({
  required String channelId,
  required String messageId,
  String? threadRootId,
}) {
  return buildMessageLink(
    channelId: channelId,
    messageId: messageId,
    threadRootId: threadRootId,
  );
}

MessageDeepLink? decodeInboxNotificationPayload(String payload) {
  final uri = Uri.tryParse(payload);
  if (uri == null) return null;
  return parseMessageDeepLink(uri);
}
