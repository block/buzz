part of '../share_sheet.dart';

sealed class _SendState {
  const _SendState();
}

class _SendIdle extends _SendState {
  const _SendIdle();
}

class _Sending extends _SendState {
  const _Sending(this.label);
  final String label;
}

class _SendFailed extends _SendState {
  const _SendFailed(this.message);
  final String message;
}

/// Uploads the photo or file once, then publishes the message. [upload] keeps the
/// uploaded descriptor so a retry only republishes.
Future<void> sendShare({
  required SharePayload payload,
  required String note,
  required ShareConversation target,
  required int createdAt,
  required ObjectRef<Map<String, Object?>?> upload,
  required VoidCallback onUploading,
}) async {
  var content = note.trim();
  final tags = <List<String>>[
    ['h', target.id],
    for (final pubkey in target.recipients) ['p', pubkey],
  ];
  if (payload.imagePath != null || payload.filePath != null) {
    if (upload.value == null) {
      onUploading();
      upload.value = await _bridge.upload();
    }
    final blob = upload.value!;
    final url = blob['url'] as String;
    // Named as the composer names a Files attachment.
    final filename = payload.filePath == null
        ? null
        : safeAttachmentFilename(payload.filename ?? 'file');
    tags.add([
      'imeta',
      'url $url',
      'm ${blob['type']}',
      'x ${blob['sha256']}',
      'size ${blob['size']}',
      if (blob['dim'] case final String dim) 'dim $dim',
      if (blob['blurhash'] case final String blurhash) 'blurhash $blurhash',
      if (filename != null) 'filename $filename',
    ]);
    final link = filename == null
        ? '![image]($url)'
        : '[${filename.replaceAllMapped(RegExp(r'[\\\[\]]'), (m) => '\\${m[0]}')}]($url)';
    content = content.isEmpty ? link : '$content\n$link';
  }
  if (content.isEmpty) throw StateError('Add a note to send.');
  await _bridge.publish(content: content, tags: tags, createdAt: createdAt);
}

/// Picker id of the New Solo chat row.
const newSoloChatId = 'new-solo';

/// What a New Solo chat send has done so far, so a retry resumes rather than
/// creating a second chat.
class SoloChatProgress {
  SoloChatProgress() : channelId = const Uuid().v4();

  final String channelId;
  bool created = false;
  bool added = false;
}

/// Creates a private Solo chat with the Bestie, as the app's New chat does,
/// and returns it as a destination for the share.
Future<ShareConversation> createSoloChat({
  required SoloChatProgress progress,
  required ({String pubkey, String name}) bestie,
  required String note,
  required int createdAt,
}) async {
  final id = progress.channelId;
  if (!progress.created) {
    await _bridge.publish(
      kind: 9007,
      content: '',
      tags: buildCreateChannelTags(
        channelId: id,
        name: startingMeChatTitle(note),
        channelType: 'stream',
        visibility: 'private',
        labels: const ['buzz.solo'],
      ),
      createdAt: createdAt,
    );
    progress.created = true;
  }
  if (!progress.added) {
    await _bridge.publish(
      kind: 9000,
      content: '',
      tags: [
        ['h', id],
        ['p', bestie.pubkey],
        ['role', 'member'],
      ],
      createdAt: createdAt,
    );
    progress.added = true;
  }
  return ShareConversation(
    id: id,
    name: bestie.name,
    isDm: false,
    isSolo: true,
    recipients: [bestie.pubkey],
    lastActivity: createdAt,
  );
}
