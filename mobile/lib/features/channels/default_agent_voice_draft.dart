import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:image_picker/image_picker.dart';
import 'package:path_provider/path_provider.dart';

import 'voice_note_recording.dart';
import '../../shared/relay/nostr_models.dart';

/// Durable unsent voice note, isolated by account, community, and recipient.
class DefaultAgentVoiceDraft {
  DefaultAgentVoiceDraft(
    this.identity, {
    Future<Directory> Function()? directory,
  }) : _directory = directory ?? getApplicationSupportDirectory;
  final Future<Directory> Function() _directory;
  final String identity;

  Future<File> _manifest() async {
    final root = await _directory();
    final directory = Directory('${root.path}/agent-voice-drafts');
    await directory.create(recursive: true);
    return File(
      '${directory.path}/${sha256.convert(utf8.encode(identity))}.json',
    );
  }

  Future<VoiceNoteRecording?> read() async {
    final manifest = await _manifest();
    if (!await manifest.exists()) return null;
    final data =
        jsonDecode(await manifest.readAsString()) as Map<String, dynamic>;
    return VoiceNoteRecording(
      // iOS can relocate the app container on update/reinstall. The audio
      // always lives beside its manifest, including drafts from older builds.
      file: XFile('${manifest.path}.m4a', mimeType: 'audio/mp4'),
      duration: Duration(milliseconds: data['duration'] as int),
      waveform: (data['waveform'] as List)
          .cast<num>()
          .map((x) => x.toDouble())
          .toList(),
    );
  }

  Future<VoiceNoteRecording> save(VoiceNoteRecording note) async {
    final manifest = await _manifest();
    final path = '${manifest.path}.m4a';
    if (note.file.path != path) await File(note.file.path).copy(path);
    final existing = await manifest.exists()
        ? jsonDecode(await manifest.readAsString()) as Map<String, dynamic>
        : <String, dynamic>{};
    final temporary = File('${manifest.path}.tmp');
    await temporary.writeAsString(
      jsonEncode({
        ...existing,
        'duration': note.duration.inMilliseconds,
        'waveform': note.waveform,
      }),
      flush: true,
    );
    await temporary.rename(manifest.path);
    return VoiceNoteRecording(
      file: XFile(path, mimeType: 'audio/mp4'),
      duration: note.duration,
      waveform: note.waveform,
    );
  }

  /// Reuse the signed event after an uncertain acknowledgement.
  Future<NostrEvent?> readEvent() async {
    final manifest = await _manifest();
    if (!await manifest.exists()) return null;
    final data =
        jsonDecode(await manifest.readAsString()) as Map<String, dynamic>;
    return data['event'] == null
        ? null
        : NostrEvent.fromJson(data['event'] as Map<String, dynamic>);
  }

  /// Persist the exact signed event before it can leave the device.
  Future<void> saveEvent(NostrEvent event) async {
    final manifest = await _manifest();
    final data =
        jsonDecode(await manifest.readAsString()) as Map<String, dynamic>;
    data['event'] = event.toJson();
    final temporary = File('${manifest.path}.tmp');
    await temporary.writeAsString(jsonEncode(data), flush: true);
    await temporary.rename(manifest.path);
  }

  Future<void> clear() async {
    final manifest = await _manifest();
    final audio = File('${manifest.path}.m4a');
    if (await audio.exists()) await audio.delete();
    if (await manifest.exists()) await manifest.delete();
  }
}

/// Injectable local draft storage for the default-agent recording surface.
final defaultAgentVoiceDraftProvider =
    Provider.family<DefaultAgentVoiceDraft, String>(
      (ref, identity) => DefaultAgentVoiceDraft(identity),
    );
