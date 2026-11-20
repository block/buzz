import 'dart:io';
import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';

/// Prepares backup exclusions before a recording can be written.
Future<Directory> privateTranscriptSupportDirectory(String directory) async {
  if (!Platform.isIOS) return getApplicationSupportDirectory();
  final path = await const MethodChannel('buzz/transcription')
      .invokeMethod<String>('storageDirectory', {
        'id': 'storage',
        'directory': directory,
      });
  if (path == null) {
    throw StateError('Private transcript storage unavailable.');
  }
  return Directory(path);
}
