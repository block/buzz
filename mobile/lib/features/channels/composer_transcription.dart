import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../../shared/transcription/private_transcript_storage.dart';
import 'package:uuid/uuid.dart';

import '../../shared/theme/theme_provider.dart';
import '../../shared/transcription/device_transcription.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/drafts/compose_drafts_provider.dart';
import 'voice_note_recording.dart';

/// Local-file boundary for retaining an unsent recording across app restarts.
class ComposerTranscriptionFiles {
  Future<String> retain(
    String source,
    Future<void> Function(String) reserve,
  ) async {
    final support = await privateTranscriptSupportDirectory(
      'composer-transcripts',
    );
    final root = Directory('${support.path}/composer-transcripts');
    await root.create(recursive: true);
    final path = '${root.path}/composer-${const Uuid().v4()}.m4a';
    await reserve(path);
    await File(source).copy(path);
    return path;
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
  const ComposerTranscription(
    this.panel,
    this.complete,
    this.isAvailable,
    this.stopping,
    this.stopFailed,
    this.pendingStop,
  );
  final Widget? panel;
  final VoidCallback stopping;
  final VoidCallback stopFailed;
  final ValueNotifier<bool> pendingStop;
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
final _pendingStopsProvider = Provider(
  (ref) => <String, ValueNotifier<bool>>{},
);
final _cleanupOperationsProvider = Provider((ref) => <String, Future<void>>{});
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
  final pendingStop = ref
      .read(_pendingStopsProvider)
      .putIfAbsent(key, () => ValueNotifier(false));
  useListenable(pendingStop);
  final ownsStop = useMemoized(() => ObjectRef<Object?>(null), [scope]);
  final cleanupOperations = ref.read(_cleanupOperationsProvider);
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

  // Serialize journal read/modify/write with sweeps from other mounts.
  Future<void> cleanupTransaction(Future<void> Function() action) {
    final previous = cleanupOperations[key] ?? Future<void>.value();
    final operation = previous.then((_) => action());
    final settled = operation.then<void>((_) {}, onError: (Object _) {});
    cleanupOperations[key] = settled;
    unawaited(
      settled.then((_) {
        if (identical(cleanupOperations[key], settled)) {
          cleanupOperations.remove(key);
        }
      }),
    );
    return operation;
  }

  List<String> cleanupPaths() {
    final journal = prefs.getString(cleanupKey);
    if (journal == null) return [];
    // Older versions stored one absolute path, without JSON encoding.
    if (!journal.startsWith('[')) return [journal];
    return (jsonDecode(journal) as List).cast<String>();
  }

  Future<void> saveCleanup(List<String> paths) async {
    final saved = paths.isEmpty
        ? await prefs.remove(cleanupKey)
        : await prefs.setString(cleanupKey, jsonEncode(paths));
    if (!saved) throw StateError('Could not schedule recording cleanup.');
  }

  Future<void> sweepUnlocked() async {
    try {
      final paths = cleanupPaths();
      for (final path in [...paths]) {
        final pointer = prefs.getString(key);
        if (pointer == path ||
            pointer == '$_pendingSourcePrefix$path' ||
            pendingSources[key] == path) {
          continue;
        }
        try {
          await files.delete(path);
        } catch (_) {
          continue; // Keep every failed deletion for a later mount.
        }
        paths.remove(path);
        await saveCleanup(paths);
      }
    } catch (_) {
      await prefs.reload();
    }
  }

  void restore() {
    final stored = prefs.getString(key);
    final source =
        pendingSources[key] ??
        (stored?.startsWith(_pendingSourcePrefix) == true
            ? stored!.substring(_pendingSourcePrefix.length)
            : null);
    savedPath.value = source ?? stored;
    retained.value = source == null;
    error.value = savedPath.value == null
        ? null
        : retained.value
        ? 'Recording ready to transcribe.'
        : 'Recording not safely saved. Retry before closing the app.';
  }

  useEffect(() {
    void changed() {
      if (!pendingStop.value && ownsStop.value == null && !busy.value) {
        restore();
      }
    }

    pendingStop.addListener(changed);
    return () => pendingStop.removeListener(changed);
  }, [scope, pendingStop]);

  useEffect(() {
    unawaited(cleanupTransaction(sweepUnlocked));
    restore();
    busy.value = false;
    retaining.value = false;
    discarding.value = false;
    return () {
      generation.value++;
      unawaited(cancelNative());
    };
  }, [scope]);

  bool current(int ticket) =>
      context.mounted && owner.value == scope && generation.value == ticket;

  Future<String> retainSource(String source) async {
    late String retainedPath;
    await cleanupTransaction(() async {
      final stored = prefs.getString(key);
      if (stored != null && !stored.startsWith(_pendingSourcePrefix)) {
        retainedPath = stored;
        return;
      }
      pendingSources[key] = source;
      try {
        if (!await prefs.setString(key, '$_pendingSourcePrefix$source')) {
          throw StateError('Could not save the recording.');
        }
        retainedPath = await files.retain(source, (destination) async {
          // Journal both paths before the copy can leave any bytes behind.
          await saveCleanup({...cleanupPaths(), source, destination}.toList());
        });
        if (!await prefs.setString(key, retainedPath)) {
          throw StateError('Could not save the recording.');
        }
        pendingSources.remove(key);
      } catch (_) {
        // SharedPreferences changes its cache even when the write fails.
        await prefs.reload();
        rethrow;
      } finally {
        await sweepUnlocked();
      }
    });
    return retainedPath;
  }

  Future<void> clear(String path) => cleanupTransaction(() async {
    final pointer = prefs.getString(key);
    final source = pendingSources[key];
    if (pointer != path &&
        pointer != '$_pendingSourcePrefix$path' &&
        !(pointer == null && source == path)) {
      return;
    }
    List<String> paths;
    try {
      paths = cleanupPaths();
      if (!paths.contains(path)) paths.add(path);
      await saveCleanup(paths);
      // Cache absence is not proof that pointer removal reached storage.
      if (!await prefs.remove(key)) {
        throw StateError('Could not clear the saved recording.');
      }
    } catch (_) {
      await prefs.reload();
      rethrow;
    }
    if (pendingSources[key] == path) pendingSources.remove(key);
    try {
      await files.delete(path);
      paths.remove(path);
      await saveCleanup(paths);
    } catch (_) {
      // The durable queue owns cleanup; keep the editor usable.
      await prefs.reload();
    }
  });

  Future<void> transcribe([
    VoiceNoteRecording? recording,
    VoidCallback? handedOff,
  ]) async {
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
      // The recording is in recovery now; recognition no longer holds Stop.
      handedOff?.call();
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
      if (!current(ticket)) return;
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
    (recording) => unawaited(() async {
      // Release only this Stop's hold, so a late completion cannot clear a
      // newer Stop's.
      final stop = ownsStop.value;
      void release() {
        if (stop == null || !identical(ownsStop.value, stop)) return;
        pendingStop.value = false;
        ownsStop.value = null;
      }

      try {
        await transcribe(recording, release);
      } finally {
        release();
      }
    }()),
    () => factory().isAvailable(),
    () {
      ownsStop.value = Object();
      pendingStop.value = true;
    },
    () {
      pendingStop.value = false;
      ownsStop.value = null;
    },
    pendingStop,
  );
}
