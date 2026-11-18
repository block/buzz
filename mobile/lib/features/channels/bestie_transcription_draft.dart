import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:path_provider/path_provider.dart';
import 'package:uuid/uuid.dart';

import '../../shared/relay/relay.dart';
import 'voice_note_recording.dart';

/// One pending utterance per account/community, retaining its original recipient.
class BestieTranscriptDraft {
  const BestieTranscriptDraft({
    required this.id,
    required this.recipient,
    required this.audioPath,
    this.text,
    this.event,
  });
  final String id;
  final String recipient;
  final String audioPath;
  final String? text;
  final NostrEvent? event;
}

final bestieTranscriptDirectoryProvider =
    Provider<Future<Directory> Function()>(
      (ref) => () async {
        if (!Platform.isIOS) return getApplicationSupportDirectory();
        final path = await const MethodChannel(
          'buzz/transcription',
        ).invokeMethod<String>('storageDirectory', {'id': 'storage'});
        if (path == null) {
          throw StateError('Private transcript storage unavailable.');
        }
        return Directory(path);
      },
    );
final bestieTranscriptStoreProvider =
    Provider.family<BestieTranscriptStore, String>(
      (ref, scope) => BestieTranscriptStore(
        scope,
        ref.watch(bestieTranscriptDirectoryProvider),
      ),
    );

/// Atomic, serialized storage. Audio is local-only and deleted on acceptance/discard.
class BestieTranscriptStore {
  BestieTranscriptStore(this.scope, this.directory);
  final String scope;
  final Future<Directory> Function() directory;
  static final _writes = <String, Future<void>>{};
  Future<T> _mutate<T>(Future<T> Function(File) action) async {
    final previous = _writes[scope];
    final done = Completer<void>();
    _writes[scope] = done.future;
    try {
      await previous;
      final root = Directory('${(await directory()).path}/bestie-transcripts');
      await root.create(recursive: true);
      return await action(
        File('${root.path}/${sha256.convert(utf8.encode(scope))}.json'),
      );
    } finally {
      done.complete();
      if (identical(_writes[scope], done.future)) _writes.remove(scope);
    }
  }

  Future<Map<String, dynamic>> _data(File file) async =>
      jsonDecode(await file.readAsString()) as Map<String, dynamic>;
  Future<void> _write(File file, Map<String, dynamic> data) async {
    final temp = File('${file.path}.tmp');
    await temp.writeAsString(jsonEncode(data), flush: true);
    await temp.rename(file.path);
  }

  BestieTranscriptDraft _decode(File file, Map<String, dynamic> data) =>
      BestieTranscriptDraft(
        id: data['id'] as String,
        recipient: data['recipient'] as String,
        // Until promotion finishes, the original recording is authoritative.
        audioPath: data['source'] as String? ?? '${file.path}.wav',
        text: data['text'] as String?,
        event: data['event'] == null
            ? null
            : NostrEvent.fromJson(data['event'] as Map<String, dynamic>),
      );
  Future<BestieTranscriptDraft?> read() => _mutate(
    (file) async =>
        await file.exists() ? _decode(file, await _data(file)) : null,
  );

  /// Reserve the slot before finalizing capture so remounts wait for recovery.
  Future<BestieTranscriptDraft> retain(
    Future<VoiceNoteRecording> recording,
    String recipient,
  ) {
    // Observe native failure immediately, even while an older disk operation
    // or the OS directory lookup still owns the serialized queue.
    VoiceNoteRecording? completed;
    Object? failure;
    StackTrace? failureStack;
    final finalization = recording.then<void>(
      (note) {
        completed = note;
      },
      onError: (Object error, StackTrace stack) {
        failure = error;
        failureStack = stack;
      },
    );
    return _mutate((file) async {
      await finalization;
      if (failure != null) Error.throwWithStackTrace(failure!, failureStack!);
      final note = completed!;
      if (await file.exists()) {
        throw StateError('A previous transcript still needs delivery.');
      }
      final source = note.file.path;
      final data = <String, dynamic>{
        'id': const Uuid().v4(),
        'recipient': recipient,
        'source': source,
      };
      // Journal first, pointing at the original: a failed or interrupted
      // promotion stays recoverable from it, and discard removes both copies.
      await _write(file, data);
      final promoted = File('${file.path}.wav.tmp');
      await File(source).copy(promoted.path);
      await promoted.rename('${file.path}.wav');
      data.remove('source');
      await _write(file, data);
      await File(source).delete();
      return _decode(file, data);
    });
  }

  Future<BestieTranscriptDraft> update(
    BestieTranscriptDraft draft, {
    String? text,
    NostrEvent? event,
  }) => _mutate((file) async {
    final data = await _data(file);
    if (data['id'] != draft.id) {
      throw StateError('The saved transcript changed.');
    }
    if (text != null) data['text'] = text;
    if (event != null) data['event'] = event.toJson();
    await _write(file, data);
    return _decode(file, data);
  });
  Future<void> clear({String? id}) => _mutate((file) async {
    if (id != null && await file.exists() && (await _data(file))['id'] != id) {
      return;
    }
    // Best effort: Discard must still remove a journal that cannot be read.
    String? source;
    try {
      if (await file.exists()) {
        source = (await _data(file))['source'] as String?;
      }
    } catch (_) {
      source = null;
    }
    for (final audio in [
      File('${file.path}.wav'),
      File('${file.path}.wav.tmp'),
      if (source != null) File(source),
    ]) {
      if (await audio.exists()) await audio.delete();
    }
    if (await file.exists()) await file.delete();
  });
}
