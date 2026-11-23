import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

const _appGroupChannel = MethodChannel('buzz/app_group');

/// Tells the iOS share extension who [pubkey]'s Bestie is, so its picker can
/// pin the Bestie DM. The extension can't decrypt the Bestie preference.
Future<void> publishShareBestie({
  required String pubkey,
  required String? bestie,
  required String? name,
}) async {
  if (!Platform.isIOS) return;
  try {
    final path = await _appGroupChannel.invokeMethod<String>('cachesDirectory');
    if (path == null) return;
    final file = File('$path/share-bestie.json');
    if (bestie == null) {
      if (await file.exists()) await file.delete();
      return;
    }
    await file.parent.create(recursive: true);
    await file.writeAsString(
      jsonEncode({'pubkey': pubkey, 'bestie': bestie, 'name': ?name}),
      flush: true,
    );
  } on Object catch (error) {
    debugPrint('[ShareBestie] could not publish: $error');
  }
}
