import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'share_targets.dart';
import 'shared_payload.dart';

const _channel = MethodChannel('buzz/share_inbox');

typedef SharedPayloadReader = Future<SharedPayload?> Function();
typedef SharedPayloadDiscarder = Future<void> Function(String id);
typedef ShareTargetsSyncer =
    Future<void> Function(ShareTargetsSnapshot snapshot);

/// Reads the newest payload staged by the Share Extension, if any.
Future<SharedPayload?> takePendingSharedPayload() async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return null;
  try {
    final raw = await _channel.invokeMapMethod<dynamic, dynamic>(
      'takePendingShare',
    );
    return SharedPayload.fromMap(raw);
  } on MissingPluginException {
    // Flutter tests and non-Runner embeddings do not install the native bridge.
    return null;
  }
}

/// Marks a payload as consumed so it is not surfaced again.
Future<void> discardSharedPayload(String id) async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  try {
    await _channel.invokeMethod<void>('discardShare', {'id': id});
  } on MissingPluginException {
    // See above.
  }
}

/// Publishes the destinations the Share Extension may offer.
Future<void> syncShareTargets(ShareTargetsSnapshot snapshot) async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  try {
    await _channel.invokeMethod<void>(
      'syncShareTargets',
      snapshot.toArguments(),
    );
  } on MissingPluginException {
    // See above.
  }
}
