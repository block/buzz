import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:image_picker/image_picker.dart';
import 'package:path_provider/path_provider.dart';

import 'voice_note_recording.dart';
import '../../shared/relay/nostr_models.dart';
import '../../shared/me/default_agent.dart';

/// Durable unsent voice note, isolated by account, community, and recipient.
class DefaultAgentVoiceDraft {
  DefaultAgentVoiceDraft(
    this.identity, {
    Future<Directory> Function()? directory,
    this.onChanged,
  }) : _directory = directory ?? getApplicationSupportDirectory;
  final Future<Directory> Function() _directory;
  final String identity;
  final void Function()? onChanged;

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
    // Keep the recipient discoverable even if the manifest becomes unreadable.
    await File(
      '${manifest.path}.identity',
    ).writeAsString(identity, flush: true);
    final path = '${manifest.path}.m4a';
    if (note.file.path != path) await File(note.file.path).copy(path);
    final existing = await manifest.exists()
        ? jsonDecode(await manifest.readAsString()) as Map<String, dynamic>
        : <String, dynamic>{};
    final temporary = File('${manifest.path}.tmp');
    await temporary.writeAsString(
      jsonEncode({
        ...existing,
        'identity': identity,
        'duration': note.duration.inMilliseconds,
        'waveform': note.waveform,
      }),
      flush: true,
    );
    await temporary.rename(manifest.path);
    onChanged?.call();
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
    final index = File('${manifest.path}.identity');
    if (await index.exists()) await index.delete();
    onChanged?.call();
  }
}

/// Injectable local draft storage for the default-agent recording surface.
final defaultAgentVoiceDraftProvider =
    Provider.family<DefaultAgentVoiceDraft, String>(
      (ref, identity) => DefaultAgentVoiceDraft(
        identity,
        directory: ref.watch(agentVoiceDraftDirectoryProvider),
        onChanged: () => ref.invalidate(pendingAgentVoiceRecipientsProvider),
      ),
    );

/// Injectable OS storage boundary shared by draft writes and discovery.
final agentVoiceDraftDirectoryProvider = Provider<Future<Directory> Function()>(
  (ref) => getApplicationSupportDirectory,
);

/// Finds pending recipients without depending on the current default agent.
/// The current recipient also locates pre-index manifests from earlier builds.
Future<List<String>> discoverAgentVoiceRecipients(
  String scopeIdentity, {
  required Future<Directory> Function() directory,
  String? currentRecipient,
}) async {
  final root = await directory();
  final folder = Directory('${root.path}/agent-voice-drafts');
  if (!await folder.exists()) return [];
  final recipients = <String>{};
  await for (final entry in folder.list()) {
    if (entry is! File || !entry.path.endsWith('.json')) continue;
    String? identity;
    try {
      final data = jsonDecode(await entry.readAsString());
      if (data is Map<String, dynamic> && data['identity'] is String) {
        identity = data['identity'] as String;
      }
    } on FormatException {
      // A damaged manifest must not hide other drafts or its separate index.
    } on FileSystemException {
      // The audio/index may still be accessible when the manifest is not.
    }
    if (identity == null) {
      try {
        final index = File('${entry.path}.identity');
        if (await index.exists()) identity = await index.readAsString();
      } on FileSystemException {
        // Preserve unidentifiable files; the current-recipient fallback below
        // can still expose their recovery/discard control without reading them.
      }
    }
    if (identity != null && identity.startsWith('$scopeIdentity:')) {
      final recipient = identity.substring(scopeIdentity.length + 1);
      if (recipient.isNotEmpty && !recipient.contains(':')) {
        recipients.add(recipient);
      }
    }
  }
  if (currentRecipient != null) {
    final legacy = File(
      '${folder.path}/${sha256.convert(utf8.encode('$scopeIdentity:$currentRecipient'))}.json',
    );
    if (await legacy.exists()) {
      // Index legacy manifests separately so discovery cannot overwrite
      // a signed event being persisted concurrently for retry.
      final index = File('${legacy.path}.identity');
      try {
        await index.writeAsString(
          '$scopeIdentity:$currentRecipient',
          flush: true,
        );
      } on FileSystemException {
        // Failure to index must not prevent recovery/discard for this recipient.
      }
      recipients.add(currentRecipient);
    }
  }
  return recipients.toList()..sort();
}

/// Refreshes after saves/discards and remote or local preference changes.
final pendingAgentVoiceRecipientsProvider =
    FutureProvider.family<List<String>, String>((ref, scopeIdentity) {
      final current = ref.watch(defaultAgentProvider).value;
      return discoverAgentVoiceRecipients(
        scopeIdentity,
        directory: ref.watch(agentVoiceDraftDirectoryProvider),
        currentRecipient: current,
      );
    });
