import 'dart:async';
import 'dart:io';

import 'package:flutter_tts/flutter_tts.dart';
import 'package:speech_to_text/speech_recognition_error.dart';
import 'package:speech_to_text/speech_recognition_result.dart';
import 'package:speech_to_text/speech_to_text.dart';

class HuddleBotVoiceException implements Exception {
  const HuddleBotVoiceException(this.message);

  final String message;

  @override
  String toString() => message;
}

abstract interface class HuddleBotVoiceEngine {
  Future<String?> transcribe();
  Future<void> stopTranscribing();
  Future<void> speak(String text);
  Future<void> stopSpeaking();
  Future<void> dispose();
}

/// Short-utterance speech recognition and system TTS for mobile bot turns.
///
/// Human-to-human audio continues over the Huddle Opus transport. The recorder
/// is paused by [HuddleController] while this engine owns the microphone.
class MobileHuddleBotVoiceEngine implements HuddleBotVoiceEngine {
  final SpeechToText _speech = SpeechToText();
  final FlutterTts _tts = FlutterTts();

  Completer<String?>? _transcription;
  String _recognizedWords = '';
  bool _speechReady = false;
  bool _disposed = false;

  @override
  Future<String?> transcribe() async {
    if (_disposed) throw StateError('Bot voice engine has been disposed');
    if (_transcription != null) {
      throw const HuddleBotVoiceException(
        'Speech recognition is already listening.',
      );
    }

    final completer = Completer<String?>();
    _transcription = completer;
    _recognizedWords = '';
    try {
      if (!_speechReady) {
        _speechReady = await _speech.initialize(
          onError: _onSpeechError,
          onStatus: _onSpeechStatus,
          options: [
            if (Platform.isAndroid) SpeechToText.androidNoBluetooth,
            if (Platform.isIOS) SpeechToText.iosNoBluetooth,
          ],
        );
      }
      if (!_speechReady) {
        throw const HuddleBotVoiceException(
          'Speech recognition is not available on this device.',
        );
      }

      await _speech.listen(
        onResult: _onSpeechResult,
        listenOptions: SpeechListenOptions(
          listenMode: ListenMode.confirmation,
          cancelOnError: true,
          partialResults: true,
          autoPunctuation: true,
          enableHapticFeedback: true,
          listenFor: Duration(seconds: 30),
          pauseFor: Duration(seconds: 2),
        ),
      );
      return await completer.future.timeout(
        const Duration(seconds: 35),
        onTimeout: () async {
          await _speech.stop();
          return _normalizedWords();
        },
      );
    } catch (error) {
      if (!completer.isCompleted) completer.completeError(error);
      rethrow;
    } finally {
      if (identical(_transcription, completer)) _transcription = null;
    }
  }

  void _onSpeechResult(SpeechRecognitionResult result) {
    _recognizedWords = result.recognizedWords;
    if (result.finalResult) _completeTranscription();
  }

  void _onSpeechStatus(String status) {
    if (status == SpeechToText.doneStatus ||
        status == SpeechToText.notListeningStatus) {
      _completeTranscription();
    }
  }

  void _onSpeechError(SpeechRecognitionError error) {
    final completer = _transcription;
    if (completer == null || completer.isCompleted) return;
    completer.completeError(
      HuddleBotVoiceException('Speech recognition failed: ${error.errorMsg}'),
    );
  }

  void _completeTranscription() {
    final completer = _transcription;
    if (completer == null || completer.isCompleted) return;
    completer.complete(_normalizedWords());
  }

  String? _normalizedWords() {
    final words = _recognizedWords.trim();
    return words.isEmpty ? null : words;
  }

  @override
  Future<void> stopTranscribing() async {
    if (_speech.isListening) await _speech.stop();
    _completeTranscription();
  }

  @override
  Future<void> speak(String text) async {
    final normalized = text.trim();
    if (_disposed || normalized.isEmpty) return;
    await _tts.awaitSpeakCompletion(true);
    if (Platform.isIOS) {
      await _tts.setIosAudioCategory(
        IosTextToSpeechAudioCategory.playAndRecord,
        [
          IosTextToSpeechAudioCategoryOptions.allowBluetooth,
          IosTextToSpeechAudioCategoryOptions.allowBluetoothA2DP,
          IosTextToSpeechAudioCategoryOptions.defaultToSpeaker,
        ],
        IosTextToSpeechAudioMode.voicePrompt,
      );
    }
    final result = await _tts.speak(normalized);
    if (result != 1) {
      throw const HuddleBotVoiceException(
        'The device text-to-speech engine could not play the bot reply.',
      );
    }
  }

  @override
  Future<void> stopSpeaking() async {
    await _tts.stop();
  }

  @override
  Future<void> dispose() async {
    if (_disposed) return;
    _disposed = true;
    await stopTranscribing();
    await stopSpeaking();
  }
}
