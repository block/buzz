import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'prepare_speech_text.dart';
import 'read_aloud_engine.dart';

/// Message id currently being spoken, or null when idle.
///
/// One utterance at a time. Starting another message stops the previous one.
final readAloudControllerProvider =
    NotifierProvider<ReadAloudController, String?>(ReadAloudController.new);

/// System TTS binding. Tests override this; production uses the preferred
/// Android/iOS engine and does not pin a package id.
final readAloudEngineProvider = Provider<ReadAloudEngine>(
  (ref) => FlutterTtsReadAloudEngine(),
);

enum ReadAloudOutcome { stopped, finished, nothingToRead, failed }

class ReadAloudController extends Notifier<String?> {
  var _generation = 0;

  @override
  String? build() => null;

  /// Speaks [content], or stops if [messageId] is the utterance in progress.
  Future<ReadAloudOutcome> toggle({
    required String messageId,
    required String content,
    List<List<String>> tags = const [],
  }) async {
    if (state == messageId) {
      await stop();
      return ReadAloudOutcome.stopped;
    }
    final spoken = prepareSpeechText(content, tags: tags);
    if (spoken.isEmpty) return ReadAloudOutcome.nothingToRead;

    final generation = ++_generation;
    final engine = ref.read(readAloudEngineProvider);
    await engine.stop();
    if (generation != _generation) return ReadAloudOutcome.stopped;
    state = messageId;
    try {
      await engine.speak(spoken);
      return ReadAloudOutcome.finished;
    } catch (_) {
      return ReadAloudOutcome.failed;
    } finally {
      if (generation == _generation && state == messageId) {
        state = null;
      }
    }
  }

  Future<void> stop() async {
    _generation++;
    state = null;
    try {
      await ref.read(readAloudEngineProvider).stop();
    } catch (_) {
      // The generation bump already drops the in-flight utterance.
    }
  }
}
