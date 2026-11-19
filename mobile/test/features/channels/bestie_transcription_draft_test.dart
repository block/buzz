import 'dart:async';
import 'dart:io';
import 'package:buzz/features/channels/bestie_transcription_draft.dart';
import 'package:buzz/features/channels/voice_note_recording.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image_picker/image_picker.dart';

void main() {
  late Directory directory;
  late BestieTranscriptStore store;
  setUp(() async {
    directory = await Directory.systemTemp.createTemp('bestie-transcript-test');
    store = BestieTranscriptStore('account:community', () async => directory);
  });
  tearDown(() async {
    await directory.delete(recursive: true);
  });
  Future<VoiceNoteRecording> recording() async {
    final file = await File(
      '${directory.path}/capture.wav',
    ).writeAsBytes([1, 2, 3]);
    return VoiceNoteRecording(
      file: XFile(file.path),
      duration: const Duration(seconds: 1),
      waveform: [.4],
    );
  }

  test(
    'finalization reserves recovery across remounts and preserves signed text',
    () async {
      final completion = Completer<VoiceNoteRecording>();
      final retaining = store.retain(completion.future, 'original-bestie');
      final remounted = BestieTranscriptStore(
        'account:community',
        () async => directory,
      );
      final recovered = remounted.read();
      completion.complete(await recording());
      final draft = await retaining;
      expect((await recovered)?.recipient, 'original-bestie');
      final event = NostrEvent(
        id: 'event',
        pubkey: 'author',
        createdAt: 1,
        kind: 9,
        tags: [
          ['h', 'conversation'],
        ],
        content: 'A normal text message',
        sig: 'signature',
      );
      await store.update(draft, text: event.content, event: event);
      final reopened = (await remounted.read())!;
      expect(reopened.text, event.content);
      expect(reopened.event?.toJson(), event.toJson());
      expect(await File(reopened.audioPath).readAsBytes(), [1, 2, 3]);
      expect(
        await BestieTranscriptStore(
          'different:community',
          () async => directory,
        ).read(),
        isNull,
      );
      await store.clear(id: draft.id);
      expect(await remounted.read(), isNull);
      expect(await File(reopened.audioPath).exists(), isFalse);
    },
  );
  test(
    'a failed audio promotion keeps the original recording recoverable',
    () async {
      final first = await store.retain(recording(), 'first');
      await store.clear(id: first.id);
      // Make promotion fail like a storage error without filling the disk.
      await Directory(first.audioPath).create();
      await expectLater(
        store.retain(recording(), 'bestie'),
        throwsA(isA<FileSystemException>()),
      );
      await Directory(first.audioPath).delete();
      final reopened = (await store.read())!;
      expect(reopened.audioPath, '${directory.path}/capture.wav');
      expect(await File(reopened.audioPath).exists(), isTrue);
      await store.clear(id: reopened.id);
      expect(await File(reopened.audioPath).exists(), isFalse);
      expect(await store.read(), isNull);
    },
  );
  for (final contents in ['{not json', '[]', '{"source": 1}']) {
    test('discard removes a journal that cannot be read: $contents', () async {
      final draft = await store.retain(recording(), 'bestie');
      final journal = File(draft.audioPath.replaceFirst(RegExp(r'\.wav$'), ''));
      await journal.writeAsString(contents);
      await store.clear();
      expect(await journal.exists(), isFalse);
      expect(await File(draft.audioPath).exists(), isFalse);
    });
  }
  test(
    'native failure is observed while directory lookup is still pending',
    () async {
      final location = Completer<Directory>();
      final delayed = BestieTranscriptStore('delayed', () => location.future);
      final capture = Completer<VoiceNoteRecording>();
      final retaining = delayed.retain(capture.future, 'bestie');
      final observed = expectLater(retaining, throwsStateError);
      capture.completeError(StateError('Interrupted capture'));
      // Cross an event-loop turn while disk lookup remains deliberately blocked.
      await Future<void>(() {});
      location.complete(directory);
      await observed;
      expect(await delayed.read(), isNull);
    },
  );
  test(
    'stale completion cannot clear or overwrite a replacement draft',
    () async {
      final old = await store.retain(recording(), 'old');
      await store.clear();
      final next = await store.retain(recording(), 'new');
      await store.clear(id: old.id);
      await expectLater(store.update(old, text: 'stale'), throwsStateError);
      expect((await store.read())?.id, next.id);
    },
  );
}
