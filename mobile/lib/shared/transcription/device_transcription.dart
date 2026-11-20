import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:uuid/uuid.dart';

/// On-device recognition of a finalized local 16 kHz mono PCM WAV file.
abstract interface class DeviceTranscription {
  Future<void> prepare();
  Future<String> transcribe(String path);
  Future<void> cancel();
}

/// Each capture owns an independent cancellation token.
final deviceTranscriptionProvider = Provider<DeviceTranscription Function()>(
  (ref) => NativeDeviceTranscription.new,
);

class NativeDeviceTranscription implements DeviceTranscription {
  static const _channel = MethodChannel('buzz/transcription');
  final _id = const Uuid().v4();
  @override
  Future<void> prepare() => _channel.invokeMethod('prepare', {'id': _id});
  @override
  Future<String> transcribe(String path) async =>
      (await _channel.invokeMethod<String>('transcribe', {
        'id': _id,
        'path': path,
      }))?.trim() ??
      '';
  @override
  Future<void> cancel() => _channel.invokeMethod('cancel', {'id': _id});
}
