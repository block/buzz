import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../deeplink/deep_link.dart';

const _channelName = 'xyz.block.buzz.mobile/notifications';

enum AndroidNotificationPermission { notDetermined, granted, denied }

@immutable
class AndroidNotificationStatus {
  const AndroidNotificationStatus({
    required this.permission,
    required this.priorityChannelEnabled,
    required this.activityChannelEnabled,
  });

  const AndroidNotificationStatus.unavailable()
    : permission = AndroidNotificationPermission.notDetermined,
      priorityChannelEnabled = false,
      activityChannelEnabled = false;

  final AndroidNotificationPermission permission;
  final bool priorityChannelEnabled;
  final bool activityChannelEnabled;

  factory AndroidNotificationStatus.fromMap(Map<Object?, Object?> value) {
    return AndroidNotificationStatus(
      permission: _parsePermission(value['permission']),
      priorityChannelEnabled: value['priorityChannelEnabled'] == true,
      activityChannelEnabled: value['activityChannelEnabled'] == true,
    );
  }
}

AndroidNotificationPermission _parsePermission(Object? value) {
  return switch (value) {
    'granted' ||
    'authorized' ||
    'notRequired' => AndroidNotificationPermission.granted,
    'denied' ||
    'blocked' ||
    'permanentlyDenied' => AndroidNotificationPermission.denied,
    _ => AndroidNotificationPermission.notDetermined,
  };
}

class AndroidNotificationBridge {
  AndroidNotificationBridge({MethodChannel? channel})
    : _channel = channel ?? const MethodChannel(_channelName) {
    _channel.setMethodCallHandler(_handleMethodCall);
  }

  final MethodChannel _channel;
  final StreamController<String> _notificationTaps =
      StreamController<String>.broadcast();
  final StreamController<AndroidNotificationStatus> _statusChanges =
      StreamController<AndroidNotificationStatus>.broadcast();

  Stream<String> get notificationTaps => _notificationTaps.stream;
  Stream<AndroidNotificationStatus> get statusChanges => _statusChanges.stream;

  Future<AndroidNotificationStatus> getStatus() async {
    final value = await _channel.invokeMethod<Map<Object?, Object?>>(
      'getStatus',
    );
    if (value == null) return const AndroidNotificationStatus.unavailable();
    return AndroidNotificationStatus.fromMap(value);
  }

  Future<AndroidNotificationStatus> requestPermission() async {
    await _channel.invokeMethod<void>('requestPermission');
    return getStatus();
  }

  Future<AndroidNotificationStatus> ensureChannels() async {
    await _channel.invokeMethod<void>('ensureChannels');
    return getStatus();
  }

  Future<void> show({
    required int id,
    required String channel,
    required String title,
    required String body,
    required String route,
  }) {
    if (_validatedNotificationRoute(route) == null) {
      throw ArgumentError.value(route, 'route', 'invalid notification route');
    }
    return _channel.invokeMethod<void>('show', <String, Object>{
      'id': id,
      'channel': channel,
      'title': title,
      'body': body,
      'route': route,
    });
  }

  Future<void> openSettings() => _channel.invokeMethod<void>('openSettings');

  Future<String?> getInitialRoute() async => _validatedNotificationRoute(
    await _channel.invokeMethod<String>('getInitialRoute'),
  );

  Future<void> _handleMethodCall(MethodCall call) async {
    switch (call.method) {
      case 'notificationTapped':
        final route = _validatedNotificationRoute(call.arguments);
        if (route != null) _notificationTaps.add(route);
      case 'notificationStatusChanged':
        final value = call.arguments;
        if (value is Map<Object?, Object?>) {
          _statusChanges.add(AndroidNotificationStatus.fromMap(value));
        }
    }
  }

  void dispose() {
    _channel.setMethodCallHandler(null);
    _notificationTaps.close();
    _statusChanges.close();
  }
}

final androidNotificationBridgeProvider = Provider<AndroidNotificationBridge>((
  ref,
) {
  final bridge = AndroidNotificationBridge();
  ref.onDispose(bridge.dispose);
  return bridge;
});

String? _validatedNotificationRoute(Object? value) {
  if (value is! String) return null;
  final uri = Uri.tryParse(value);
  if (uri == null) return null;
  final link = parseMessageDeepLink(uri);
  if (link == null) return null;

  const allowedKeys = {'channel', 'id', 'thread'};
  if (uri.queryParametersAll.entries.any(
    (entry) =>
        !allowedKeys.contains(entry.key) ||
        entry.value.length != 1 ||
        entry.value.single.isEmpty,
  )) {
    return null;
  }

  final canonical = buildMessageLink(
    channelId: link.channelId,
    messageId: link.messageId,
    threadRootId: link.threadRootId,
  );
  return value == canonical ? value : null;
}
