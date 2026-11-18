import 'dart:async';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:path_provider/path_provider.dart';
import 'package:uuid/uuid.dart';

import '../../shared/theme/theme_provider.dart';
import '../../shared/transcription/device_transcription.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../activity/compose_drafts_provider.dart';
import 'voice_note_recording.dart';

/// Local-file boundary for retaining an unsent recording across app restarts.
class ComposerTranscriptionFiles {
  Future<String> retain(String source) async {
    final root = await getApplicationSupportDirectory();
    final path = '${root.path}/composer-${const Uuid().v4()}.m4a';
    await File(source).copy(path);
    return path;
  }

  Future<void> delete(String path) async {
    final file = File(path);
    if (await file.exists()) await file.delete();
  }
}

final composerTranscriptionFilesProvider = Provider<ComposerTranscriptionFiles>(
  (ref) => ComposerTranscriptionFiles(),
);

class ComposerTranscription {
  const ComposerTranscription(this.panel, this.complete, this.isAvailable);
  final Widget? panel;
  final ValueChanged<VoiceNoteRecording> complete;
  final Future<bool> Function() isAvailable;
}

/// Recognition owns only a local recording; it never publishes a message.
ComposerTranscription useComposerTranscription({
  required BuildContext context,
  required WidgetRef ref,
  required String scope,
  required ValueChanged<String> onText,
}) {
  final prefs = ref.read(savedPrefsProvider);
  final key = 'composer_transcription:$scope';
  final savedPath = useState<String?>(null);
  final busy = useState(false);
  final retaining = useState(false);
  final discarding = useRef(false);
  final insertedPath = useRef<String?>(null);
  final error = useState<String?>(null);
  final generation = useRef(0);
  final speech = useRef<DeviceTranscription?>(null);
  final owner = useRef(scope)..value = scope;
  final insert = useRef(onText)..value = onText;
  final files = ref.read(composerTranscriptionFilesProvider);
  final factory = ref.read(deviceTranscriptionProvider);

  Future<void> cancelNative() async {
    final active = speech.value;
    speech.value = null;
    try {
      await active?.cancel();
    } catch (_) {
      // A completed or disconnected platform operation no longer owns input.
    }
  }

  useEffect(() {
    savedPath.value = prefs.getString(key);
    busy.value = false;
    retaining.value = false;
    discarding.value = false;
    insertedPath.value = null;
    error.value = savedPath.value == null
        ? null
        : 'Recording ready to transcribe.';
    return () {
      generation.value++;
      unawaited(cancelNative());
    };
  }, [scope]);

  bool current(int ticket) =>
      context.mounted && owner.value == scope && generation.value == ticket;

  Future<void> clear(String path) async {
    if (prefs.getString(key) == path && !await prefs.remove(key)) {
      throw StateError('Could not clear the saved recording.');
    }
    try {
      await files.delete(path);
    } catch (_) {
      // The draft owns the text now; a failed local file cleanup cannot reinsert it.
    }
  }

  Future<void> transcribe([VoiceNoteRecording? recording]) async {
    if (busy.value) return;
    final ticket = ++generation.value;
    busy.value = true;
    error.value = null;
    var path = savedPath.value;
    try {
      if (recording != null) {
        retaining.value = true;
        // Remember the original before copying so a storage error is retryable.
        path = recording.file.path;
        savedPath.value = path;
        final retained = await files.retain(path);
        if (!await prefs.setString(key, retained)) {
          throw StateError('Could not save the recording.');
        }
        final original = path;
        path = retained;
        try {
          await files.delete(original);
        } catch (_) {
          /* Retained copy owns recovery. */
        }
        if (current(ticket)) {
          savedPath.value = path;
          retaining.value = false;
        }
      }
      if (path == null || !current(ticket)) return;
      if (insertedPath.value != path) {
        final native = factory();
        speech.value = native;
        final text = await native
            .transcribe(path)
            .timeout(const Duration(seconds: 65));
        if (!current(ticket)) return;
        if (text.trim().isEmpty) {
          throw StateError(
            'No speech recognized. Retry or discard the recording.',
          );
        }
        insert.value(text.trim());
        insertedPath.value = path;
      }
      // The editor updates its draft synchronously, then saves it asynchronously.
      // Keep the recording until that write has succeeded.
      if (!await ref.read(composeDraftsProvider.notifier).ensurePersisted()) {
        throw StateError('Could not save the draft. Your recording is saved.');
      }
      await clear(path);
      if (current(ticket)) savedPath.value = null;
    } catch (failure) {
      if (current(ticket)) {
        error.value = failure is StateError
            ? failure.message
            : 'Could not transcribe. Your recording is saved; try again.';
      }
    } finally {
      if (current(ticket)) {
        await cancelNative();
        if (current(ticket)) {
          busy.value = false;
          retaining.value = false;
        }
      }
    }
  }

  Future<void> discard() async {
    final path = savedPath.value;
    if (path == null || retaining.value || discarding.value) return;
    final ticket = ++generation.value;
    discarding.value = true;
    busy.value = true;
    try {
      await cancelNative();
      await clear(path);
      if (current(ticket)) {
        savedPath.value = null;
        error.value = null;
      }
    } catch (_) {
      if (current(ticket)) {
        error.value = 'Could not discard the recording. Try again.';
      }
    } finally {
      if (current(ticket)) {
        discarding.value = false;
        busy.value = false;
      }
    }
  }

  return ComposerTranscription(
    busy.value || savedPath.value != null
        ? Semantics(
            liveRegion: true,
            child: Row(
              children: [
                IconButton(
                  tooltip: 'Discard recording',
                  onPressed: retaining.value || discarding.value
                      ? null
                      : discard,
                  icon: const Icon(Icons.close),
                ),
                Expanded(
                  child: Text(
                    discarding.value
                        ? 'Discarding…'
                        : busy.value
                        ? 'Transcribing…'
                        : error.value ?? 'Recording saved.',
                  ),
                ),
                if (busy.value)
                  Padding(
                    padding: const EdgeInsets.all(12),
                    child: BuzzLoadingIndicator(
                      size: 20,
                      semanticLabel: discarding.value
                          ? 'Discarding'
                          : 'Transcribing',
                    ),
                  )
                else
                  TextButton(onPressed: transcribe, child: const Text('Retry')),
              ],
            ),
          )
        : null,
    (recording) => unawaited(transcribe(recording)),
    () => factory().isAvailable(),
  );
}
