import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../shared/theme/theme.dart';
import 'channel.dart';

/// Presents an entirely UIKit channel form; relay writes stay in Dart.
class IosCreateChannelSheet {
  static const _channel = MethodChannel('buzz/create_channel_sheet');
  static bool _open = false;

  static Future<Channel?> present({
    required BuildContext context,
    required Future<Channel> Function({
      required String name,
      required String description,
      required String visibility,
      required int? ttlSeconds,
    })
    onCreate,
  }) async {
    if (_open) return null;
    _open = true;
    Channel? created;
    final session = UniqueKey().toString();
    final theme = utilitySurfaceThemeData(Theme.of(context));
    _channel.setMethodCallHandler((call) async {
      final data = Map<Object?, Object?>.from(call.arguments as Map);
      if (call.method != 'create' || data['session'] != session) return null;
      try {
        created = await onCreate(
          name: data['name'] as String,
          description: data['description'] as String,
          visibility: data['visibility'] as String,
          ttlSeconds: data['ttlSeconds'] as int?,
        );
        return {'success': true};
      } catch (error) {
        return {'error': error.toString()};
      }
    });
    try {
      await _channel.invokeMethod<void>('present', {
        'session': session,
        'dark': theme.brightness == Brightness.dark,
        'pageColor': theme.colorScheme.surface.toARGB32(),
        'rowColor': theme.colorScheme.surfaceContainerHighest.toARGB32(),
      });
      return created;
    } finally {
      _channel.setMethodCallHandler(null);
      _open = false;
    }
  }
}
