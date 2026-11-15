import 'dart:async';
import 'dart:io';
import 'package:buzz/shared/relay/relay.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'dart:convert';

import 'package:buzz/features/channels/default_agent_voice_draft.dart';
import 'package:buzz/features/channels/voice_note_recording.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image_picker/image_picker.dart';

void main() {
  test(
    'restores audio and the same signed event across retry and restart',
    () async {
      var directory = await Directory.systemTemp.createTemp(
        'agent-voice-draft-test',
      );
      addTearDown(() => directory.delete(recursive: true));
      final source = File('${directory.path}/recording.m4a');
      await source.writeAsBytes([1, 2, 3]);
      final draft = DefaultAgentVoiceDraft(
        'community:account:agent',
        directory: () async => directory,
      );
      final retained = await draft.save(
        VoiceNoteRecording(
          file: XFile(source.path),
          duration: const Duration(seconds: 2),
          waveform: const [.2, .8],
        ),
      );
      final event = NostrEvent(
        id: 'a' * 64,
        pubkey: 'b' * 64,
        createdAt: 1,
        kind: 9,
        tags: const [
          ['h', 'agent-chat'],
        ],
        content: 'voice',
        sig: 'c' * 128,
      );
      expect(
        await File(
          '${retained.file.path.replaceFirst(RegExp(r'\.m4a$'), '')}.identity',
        ).readAsString(),
        'community:account:agent',
      );
      await draft.saveEvent(event);
      await draft.save(retained);
      await source.delete();
      // Updating an iOS app relocates its data container, including the draft.
      directory = await directory.rename('${directory.path}-relocated');
      final restored = DefaultAgentVoiceDraft(
        'community:account:agent',
        directory: () async => directory,
      );
      expect(await (await restored.read())!.file.readAsBytes(), [1, 2, 3]);
      expect((await restored.readEvent())!.toJson(), event.toJson());
      final other = DefaultAgentVoiceDraft(
        'other:account:agent',
        directory: () async => directory,
      );
      expect(await other.read(), isNull);
      // Upgrade a pre-index manifest while its original recipient is selected.
      final manifest = File(
        (await restored.read())!.file.path.replaceFirst(RegExp(r'\.m4a$'), ''),
      );
      final legacy =
          jsonDecode(await manifest.readAsString()) as Map<String, dynamic>;
      legacy.remove('identity');
      await manifest.writeAsString(jsonEncode(legacy));
      expect(
        await discoverAgentVoiceRecipients(
          'community:account',
          directory: () async => directory,
          currentRecipient: 'agent',
        ),
        ['agent'],
      );
      expect((await restored.readEvent())!.toJson(), event.toJson());
      // A remote default change or deleted agent must not hide this draft.
      expect(
        await discoverAgentVoiceRecipients(
          'community:account',
          directory: () async => directory,
          currentRecipient: 'other-agent',
        ),
        ['agent'],
      );
      expect(
        await discoverAgentVoiceRecipients(
          'other:account',
          directory: () async => directory,
        ),
        isEmpty,
      );
      expect(
        await discoverAgentVoiceRecipients(
          'community:other-account',
          directory: () async => directory,
        ),
        isEmpty,
      );
      final firstGeneration = (await restored.read())!.draftGeneration!;
      await restored.clear();
      final replacementSource = File('${directory.path}/replacement.m4a');
      await replacementSource.writeAsBytes([4, 5, 6]);
      final replacement = await restored.save(
        VoiceNoteRecording(
          file: XFile(replacementSource.path),
          duration: const Duration(seconds: 3),
          waveform: const [.6],
        ),
      );
      expect(replacement.draftGeneration, isNot(firstGeneration));
      await draft.clear(generation: firstGeneration);
      await expectLater(
        draft.saveEvent(event, generation: firstGeneration),
        throwsStateError,
      );
      expect(await restored.readEvent(), isNull);
      expect(await (await restored.read())!.file.readAsBytes(), [4, 5, 6]);
      final retried = await restored.save(replacement);
      expect(retried.draftGeneration, replacement.draftGeneration);
      final relocatedAudio = (await restored.read())!.file.path;
      // A corrupt sibling must not prevent discovery of a healthy draft.
      final unknown = File('${manifest.parent.path}/unknown.json');
      await unknown.writeAsString('{');
      expect(
        await discoverAgentVoiceRecipients(
          'community:account',
          directory: () async => directory,
        ),
        ['agent'],
      );
      // Separate identity metadata keeps damaged manifests discardable even
      // after the original recipient is no longer the default.
      for (final damaged in ['{', '[]', '{"identity":42}']) {
        await manifest.writeAsString(damaged);
        expect(
          await discoverAgentVoiceRecipients(
            'community:account',
            directory: () async => directory,
            currentRecipient: 'other-agent',
          ),
          ['agent'],
        );
      }
      // A crash after copying audio but before manifest rename is discoverable
      // even after the user selected another Bestie. Do not treat it as empty.
      await manifest.delete();
      expect(
        await discoverAgentVoiceRecipients(
          'community:account',
          directory: () async => directory,
          currentRecipient: 'other-agent',
        ),
        ['agent'],
      );
      await expectLater(restored.read(), throwsFormatException);
      await manifest.writeAsString('{');
      // A pre-index corrupt draft is still reachable by its known filename.
      await File('${manifest.path}.identity').delete();
      expect(
        await discoverAgentVoiceRecipients(
          'community:account',
          directory: () async => directory,
          currentRecipient: 'agent',
        ),
        ['agent'],
      );
      expect(await unknown.readAsString(), '{');
      await restored.clear();
      expect(await restored.read(), isNull);
      expect(
        await discoverAgentVoiceRecipients(
          'community:account',
          directory: () async => directory,
        ),
        isEmpty,
      );
      expect(await File(relocatedAudio).exists(), isFalse);
      // Reservation starts at release, before native stop completes. A new
      // control's restore waits for that note instead of observing an empty
      // slot and allowing a replacement to overwrite it.
      final nativeStop = Completer<VoiceNoteRecording>();
      final finalization = restored.finish(nativeStop.future);
      final remountRead = other.read(); // unrelated scope never blocks
      expect(await remountRead, isNull);
      final recovery = restored.read();
      final released = File('${directory.path}/released.m4a');
      await released.writeAsBytes([7, 8, 9]);
      nativeStop.complete(
        VoiceNoteRecording(
          file: XFile(released.path),
          duration: const Duration(seconds: 2),
          waveform: const [.5],
        ),
      );
      final completed = await finalization;
      final afterRemount = await recovery;
      expect(afterRemount!.draftGeneration, completed!.draftGeneration);
      expect(await afterRemount.file.readAsBytes(), [7, 8, 9]);
      expect(await released.exists(), isFalse);
      await restored.clear();
    },
  );
  test(
    'expired retry reconciles acceptance before renewing durable signed bytes',
    () async {
      final directory = await Directory.systemTemp.createTemp('voice-retry');
      addTearDown(() => directory.delete(recursive: true));
      final source = File('${directory.path}/source.m4a');
      await source.writeAsBytes([1, 2, 3]);
      final draft = DefaultAgentVoiceDraft(
        'scope:account:agent',
        directory: () async => directory,
      );
      for (final accepted in [false, true]) {
        final note = await draft.save(
          VoiceNoteRecording(
            file: XFile(source.path),
            duration: const Duration(seconds: 2),
            waveform: const [.5],
          ),
        );
        final old = NostrEvent(
          id: 'a' * 64,
          pubkey: 'b' * 64,
          createdAt: 1,
          kind: 9,
          tags: const [
            ['h', 'dm'],
          ],
          content: 'voice',
          sig: 'c' * 128,
        );
        await draft.saveEvent(old, generation: note.draftGeneration);
        final relay = _ExpiredRelay(old, accepted);
        final signer = SignedEventRelay(
          session: relay,
          nsec: nostr.Nip19.encode(
            prefix: nostr.Nip19Prefix.nsec,
            data: '0' * 63 + '1',
          ),
        );
        // A failed reconciliation must not authorize a second event.
        relay.queryFails = true;
        await expectLater(
          draft.retryEvent(
            session: relay,
            signer: signer,
            generation: note.draftGeneration,
            validate: () {},
          ),
          throwsStateError,
        );
        expect((await draft.readEvent())!.id, old.id);
        relay.queryFails = false;
        relay.beforePublish = (event) async {
          final manifest =
              (await Directory(
                '${directory.path}/agent-voice-drafts',
              ).list().toList()).whereType<File>().singleWhere(
                (file) => file.path.endsWith('.json'),
              );
          expect(
            (jsonDecode(await manifest.readAsString())['event'] as Map)['id'],
            event.id,
          );
        };
        await draft.retryEvent(
          session: relay,
          signer: signer,
          generation: note.draftGeneration,
          validate: () {},
        );
        expect(relay.delivered, hasLength(1));
        expect(relay.delivered.single.id == old.id, accepted);
        expect((await draft.readEvent())!.id, relay.delivered.single.id);
        // A lost acknowledgement of the renewed event retries those same bytes.
        if (!accepted) {
          await draft.retryEvent(
            session: relay,
            signer: signer,
            generation: note.draftGeneration,
            validate: () {},
          );
          expect(relay.delivered, hasLength(1));
        }
        await draft.clear();
      }
    },
  );
}

class _ExpiredRelay extends RelaySessionNotifier {
  _ExpiredRelay(this.original, bool accepted) {
    if (accepted) delivered.add(original);
  }
  final NostrEvent original;
  final delivered = <NostrEvent>[];
  bool queryFails = false;
  Future<void> Function(NostrEvent)? beforePublish;
  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (event.id == original.id) {
      throw StateError('invalid: event timestamp too far from server time');
    }
    await beforePublish?.call(event);
    if (!delivered.any((saved) => saved.id == event.id)) delivered.add(event);
    return event;
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (queryFails) throw StateError('offline');
    expect(filters.single.ids, [original.id]);
    return delivered.where((event) => event.id == original.id).toList();
  }
}
