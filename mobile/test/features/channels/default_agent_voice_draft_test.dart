import 'dart:io';
import 'dart:convert';

import 'package:buzz/features/channels/default_agent_voice_draft.dart';
import 'package:buzz/features/channels/voice_note_recording.dart';
import 'package:buzz/shared/relay/nostr_models.dart';
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
      final relocatedAudio = (await restored.read())!.file.path;
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
    },
  );
}
