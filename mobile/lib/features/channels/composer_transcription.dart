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
import '../../shared/drafts/compose_drafts_provider.dart';
import 'voice_note_recording.dart';

/// Local-file boundary for retaining an unsent recording across app restarts.
class ComposerTranscriptionFiles {
  Future<String> retain(String source) async {
    final root = await getApplicationSupportDirectory();
    final path = '${root.path}/composer-${const Uuid().v4()}.m4a';
    try {
      await File(source).copy(path);
      return path;
    } catch (_) {
      try {
        await delete(path);
      } catch (_) {
        /* Preserve the copy error if partial-file cleanup fails. */
      }
      rethrow;
    }
  }

  Future<void> delete(String path) async {
    final file = File(path);
    if (await file.exists()) await file.delete();
  }
}

/// File operations used to retain and discard pending dictation audio.
final composerTranscriptionFilesProvider = Provider<ComposerTranscriptionFiles>(
  (ref) => ComposerTranscriptionFiles(),
);

/// Controls and completion callback for one composer’s dictation workflow.
class ComposerTranscription {
  const ComposerTranscription(this.panel, this.complete, this.isAvailable);
  final Widget? panel;
  final ValueChanged<VoiceNoteRecording> complete;
  final Future<bool> Function() isAvailable;
}

// Keep a failed initial save retryable across navigation within this app session.
// This is deliberately not advertised as surviving a restart.
final _pendingTranscriptionSourcesProvider = Provider(
  (ref) => <String, String>{},
);
final _retentionOperationsProvider = Provider(
  (ref) => <String, Future<String>>{},
);
const _pendingSourcePrefix = 'pending-source:';

/// Recognition owns only a local recording; it never publishes a message.
ComposerTranscription useComposerTranscription({
  required BuildContext context,
  required WidgetRef ref,
  required String scope,
  required String draftKey,
  required void Function(String text, String recordingId) onText,
}) {
  final prefs = ref.read(savedPrefsProvider);
  final key = 'composer_transcription:$scope';
  final cleanupKey = 'composer_transcription_cleanup:$scope';
  final savedPath = useState<String?>(null);
  final busy = useState(false);
  final retaining = useState(false);
  final discarding = useRef(false);
  final retained = useRef(false);
  final pendingSources = ref.read(_pendingTranscriptionSourcesProvider);
  final retentionOperations = ref.read(_retentionOperationsProvider);
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

  // A journaled path whose pointer is gone has no decision left for the user;
  // delete it quietly. Never touch a file the pointer still names.
  Future<void> sweepCleanup() async {
    final journal = prefs.getString(cleanupKey);
    final pointer = prefs.getString(key);
    if (journal == null ||
        pointer == journal ||
        pointer == '$_pendingSourcePrefix$journal' ||
        pendingSources[key] == journal) {
      return;
    }
    try {
      await files.delete(journal);
      if (prefs.getString(cleanupKey) == journal) {
        await prefs.remove(cleanupKey);
      }
    } catch (_) {
      await prefs.reload();
      // The journal stays; a later mount retries.
    }
  }

  useEffect(() {
    unawaited(sweepCleanup());
    final stored = prefs.getString(key);
    final source =
        pendingSources[key] ??
        (stored?.startsWith(_pendingSourcePrefix) == true
            ? stored!.substring(_pendingSourcePrefix.length)
            : null);
    savedPath.value = source ?? stored;
    retained.value = source == null;
    busy.value = false;
    retaining.value = false;
    discarding.value = false;
    error.value = savedPath.value == null
        ? null
        : retained.value
        ? 'Recording ready to transcribe.'
        : 'Recording not safely saved. Retry before closing the app.';
    return () {
      generation.value++;
      unawaited(cancelNative());
    };
  }, [scope]);

  bool current(int ticket) =>
      context.mounted && owner.value == scope && generation.value == ticket;

  Future<String> retainSource(String source) async {
    // A previous mount may have completed retention since this hook read
    // the source journal. Adopt its copy instead of copying deleted audio.
    final stored = prefs.getString(key);
    if (stored != null && !stored.startsWith(_pendingSourcePrefix)) {
      return stored;
    }
    pendingSources[key] = source;
    try {
      if (!await prefs.setString(key, '$_pendingSourcePrefix$source')) {
        throw StateError('Could not save the recording.');
      }
      final copy = await files.retain(source);
      try {
        if (!await prefs.setString(key, copy)) {
          throw StateError('Could not save the recording.');
        }
      } catch (_) {
        await files.delete(copy);
        rethrow;
      }
      pendingSources.remove(key);
      try {
        await files.delete(source);
      } catch (_) {
        /* The retained copy owns recovery. */
      }
      return copy;
    } catch (_) {
      await prefs.reload();
      rethrow;
    }
  }

  Future<void> clear(String path) async {
    // remove() mutates the cache even when the platform write fails. Always
    // attempt the durable removal; never use cache absence as proof of success.
    final older = prefs.getString(cleanupKey);
    if (older != null && older != path) await sweepCleanup();
    try {
      // Keep a cleanup journal until both pointer removal and file deletion
      // succeed, including a process exit between those steps.
      if (!await prefs.setString(cleanupKey, path)) {
        throw StateError('Could not schedule recording cleanup.');
      }
      if (!await prefs.remove(key)) {
        throw StateError('Could not clear the saved recording.');
      }
    } catch (_) {
      await prefs.reload();
      rethrow;
    }
    pendingSources.remove(key);
    try {
      await files.delete(path);
    } catch (_) {
      // The journal is the durable retry record; a later mount sweeps it.
      return;
    }
    // The pointer and file are gone; a leftover journal is swept on a later
    // mount, so it must not hold the editor behind the panel.
    try {
      if (!await prefs.remove(cleanupKey)) await prefs.reload();
    } catch (_) {
      await prefs.reload();
    }
  }

  Future<void> transcribe([VoiceNoteRecording? recording]) async {
    // Stop may finish after the original editor was removed or changed scope.
    // Its captured storage dependencies still own recovery; no UI/ref access.
    if (recording != null && (!context.mounted || owner.value != scope)) {
      final source = recording.file.path;
      pendingSources[key] = source;
      final operation = retentionOperations.putIfAbsent(
        key,
        () => retainSource(source),
      );
      try {
        await operation;
      } catch (_) {
        // The source journal (or session fallback) keeps this retryable.
      } finally {
        if (identical(retentionOperations[key], operation)) {
          retentionOperations.remove(key);
        }
      }
      return;
    }
    if (busy.value) return;
    final ticket = ++generation.value;
    busy.value = true;
    error.value = null;
    var path = savedPath.value;
    try {
      if (recording != null) {
        path = recording.file.path;
        savedPath.value = path;
        retained.value = false;
        pendingSources[key] = path;
      }
      if (path == null || !current(ticket)) return;
      if (!retained.value) {
        retaining.value = true;

        final operation = retentionOperations.putIfAbsent(
          key,
          () => retainSource(path!),
        );
        try {
          path = await operation;
          if (!current(ticket)) return;
          savedPath.value = path;
          retained.value = true;
          retaining.value = false;
        } catch (_) {
          await prefs.reload();
          throw StateError(
            'Recording not safely saved. Retry before closing the app.',
          );
        } finally {
          if (identical(retentionOperations[key], operation)) {
            retentionOperations.remove(key);
          }
        }
      }
      if (!current(ticket)) return;
      final drafts = ref.read(composeDraftsProvider.notifier);
      if (drafts.draftFor(draftKey)?.transcriptionId != path) {
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
        insert.value(text.trim(), path);
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
        error.value = !retained.value
            ? 'Recording not safely saved. Retry before closing the app.'
            : failure is StateError
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
      final operation = retentionOperations[key];
      final stored = prefs.getString(key);
      final discardPath = operation != null
          ? await operation
          : !retained.value &&
                stored != null &&
                !stored.startsWith(_pendingSourcePrefix)
          ? stored
          : path;
      await clear(discardPath);
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
