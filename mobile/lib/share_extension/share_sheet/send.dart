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

/// Uploads each attachment once, then publishes one message with all of
/// them, as the composer sends it. [uploads] keeps the uploaded descriptors
/// so a retry uploads only what's left and republishes.
Future<void> sendShare({
  required SharePayload payload,
  required String note,
  required ShareConversation target,
  required int createdAt,
  required Map<int, BlobDescriptor> uploads,
  required void Function(int index) onUploading,
}) async {
  final attachments = payload.attachments;
  for (var i = 0; i < attachments.length; i++) {
    if (uploads.containsKey(i)) continue;
    onUploading(i);
    uploads[i] = await _upload(i, attachments[i]);
  }
  final blobs = [for (var i = 0; i < attachments.length; i++) uploads[i]!];
  final text = note.trim();
  final content = [
    if (text.isNotEmpty) text,
    for (final blob in blobs) blob.toMarkdownImage(),
  ].join('\n');
  if (content.isEmpty) throw StateError('Add a note to send.');
  await _bridge.publish(
    content: content,
    tags: [
      ['h', target.id],
      for (final pubkey in target.recipients) ['p', pubkey],
      for (final blob in blobs) blob.toImetaTag(),
    ],
    createdAt: createdAt,
  );
}

Future<BlobDescriptor> _upload(int index, ShareAttachment attachment) async {
  if (attachment.kind == ShareAttachmentKind.gif) {
    // The composer strips an animated image's metadata in Dart before it
    // uploads; the extension does the same, in place.
    final file = File(attachment.path);
    try {
      final clean = sanitizeAnimatedImageForUpload(
        await file.readAsBytes(),
        attachment.mime,
      );
      await file.writeAsBytes(clean, flush: true);
    } on FormatException {
      throw StateError("We couldn't prepare this GIF for upload.");
    }
  }
  final raw = await _bridge.upload(index);
  final blob = BlobDescriptor.fromJson({
    'uploaded': DateTime.now().millisecondsSinceEpoch ~/ 1000,
    ...raw,
  });
  return switch (attachment.filename) {
    final name? => blob.withFilename(safeAttachmentFilename(name)),
    null => blob,
  };
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
