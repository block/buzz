import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_tts/flutter_tts.dart';

import 'prepare_speech_text.dart';

/// Playback speed for read aloud. Marmalade has no rate control of its own.
const readAloudSpeed = 1.3;

/// flutter_tts treats 0.5 as 1.0x. Android multiplies the value by 2 before
/// `TextToSpeech.setSpeechRate`, so 0.65 is 1.3x there and on iOS, where 0.5
/// is `AVSpeechUtteranceDefaultSpeechRate`.
double flutterTtsRateForMultiplier(double multiplier) => 0.5 * multiplier;

/// Prefer a voice whose name contains "michael". Returns null when the
/// engine does not list one, in which case the system default is used.
Map<String, String>? preferredReadAloudVoice(List<Map<String, String>> voices) {
  Map<String, String>? match;
  for (final voice in voices) {
    final name = voice['name'];
    final locale = voice['locale'];
    if (name == null || locale == null) continue;
    if (!name.toLowerCase().contains('michael')) continue;
    match ??= voice;
    if (voice['network_required'] != '1') return voice;
  }
  return match;
}

/// Speaks already-cleaned text through whatever engine the platform provides.
abstract class ReadAloudEngine {
  Future<void> speak(String text);

  Future<void> stop();
}

/// Android/iOS system TTS. Does not select an engine or voice: the preferred
/// system engine (Marmalade, when the user set it) speaks.
class FlutterTtsReadAloudEngine implements ReadAloudEngine {
  FlutterTtsReadAloudEngine({FlutterTts? tts}) : _tts = tts ?? FlutterTts();

  final FlutterTts _tts;
  var _configured = false;
  var _generation = 0;

  @override
  Future<void> speak(String text) async {
    final generation = ++_generation;
    await _ensureConfigured();
    if (generation != _generation) return;
    final chunks = splitSpeechChunks(text, maxLength: await _inputLimit());
    for (final chunk in chunks) {
      if (generation != _generation) return;
      await _speakChunk(chunk, generation);
    }
  }

  @override
  Future<void> stop() async {
    _generation++;
    // Drop handlers before stop so a late cancel from this utterance cannot
    // complete the next one.
    _tts
      ..setStartHandler(() {})
      ..setCompletionHandler(() {})
      ..setCancelHandler(() {})
      ..setErrorHandler((_) {});
    try {
      await _tts.stop();
    } catch (error, stack) {
      debugPrint('read-aloud stop failed: $error\n$stack');
    }
  }

  Future<void> _ensureConfigured() async {
    if (_configured) return;
    _configured = true;
    // speak() should return when the utterance is queued. Completion is
    // observed through the handlers so an engine error cannot leave the
    // future pending.
    await _tts.awaitSpeakCompletion(false);
    await _tts.setSpeechRate(flutterTtsRateForMultiplier(readAloudSpeed));
    await _preferMichael();
  }

  Future<void> _preferMichael() async {
    try {
      final raw = await _tts.getVoices;
      if (raw is! List) return;
      final voices = <Map<String, String>>[
        for (final item in raw)
          if (item is Map)
            {
              for (final entry in item.entries)
                entry.key.toString(): entry.value.toString(),
            },
      ];
      final voice = preferredReadAloudVoice(voices);
      if (voice == null) return;
      await _tts.setVoice({'name': voice['name']!, 'locale': voice['locale']!});
    } catch (error, stack) {
      debugPrint('read-aloud voice preference skipped: $error\n$stack');
    }
  }

  Future<int> _inputLimit() async {
    if (kIsWeb || !Platform.isAndroid) return 0;
    try {
      final limit = await _tts.getMaxSpeechInputLength;
      return limit ?? 0;
    } catch (error, stack) {
      debugPrint('read-aloud input limit unavailable: $error\n$stack');
      return 0;
    }
  }

  Future<void> _speakChunk(String chunk, int generation) async {
    final done = Completer<void>();
    Object? error;
    // speak() returns when the utterance is queued, not when it finishes.
    // Ignore callbacks until then so a cancel still in flight from stop()
    // cannot retire this chunk.
    var playing = false;
    void finish() {
      if (!playing || done.isCompleted) return;
      done.complete();
    }

    _tts
      ..setCompletionHandler(finish)
      ..setCancelHandler(finish)
      ..setErrorHandler((message) {
        error = message;
        finish();
      });
    final started = await _tts
        .speak(chunk, focus: true)
        .timeout(const Duration(seconds: 60));
    if (generation != _generation) return;
    if (started == 0) {
      throw ReadAloudSpeakException('TTS rejected the utterance');
    }
    playing = true;
    if (error != null) {
      throw ReadAloudSpeakException(error.toString());
    }
    await done.future.timeout(const Duration(minutes: 3));
    if (generation != _generation) return;
    if (error != null) {
      throw ReadAloudSpeakException(error.toString());
    }
  }
}

class ReadAloudSpeakException implements Exception {
  ReadAloudSpeakException(this.message);

  final String message;

  @override
  String toString() => message;
}
