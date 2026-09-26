import 'package:flutter/services.dart';

final class HuddleSpeech {
  final MethodChannel _channel = const MethodChannel('buzz/huddle_speech');
  void Function(String)? onTranscript;
  void Function(String)? onError;

  HuddleSpeech() {
    _channel.setMethodCallHandler((call) async {
      final values = call.arguments as Map?;
      if (call.method == 'transcript') {
        final text = values?['text'];
        if (text is String && text.trim().isNotEmpty) {
          onTranscript?.call(text.trim());
        }
      } else if (call.method == 'error') {
        final message = values?['message'];
        if (message is String && message.trim().isNotEmpty) {
          onError?.call(message.trim());
        }
      }
    });
  }

  Future<void> start() => _channel.invokeMethod<void>('start');
  Future<void> stop() => _channel.invokeMethod<void>('stop');
  Future<void> speak(String text) =>
      _channel.invokeMethod<void>('speak', {'text': text});

  void dispose() {
    onTranscript = null;
    onError = null;
    _channel.setMethodCallHandler(null);
  }
}
