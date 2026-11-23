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

/// Uploads the photo once, then publishes the message. [upload] keeps the
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
  if (payload.imagePath != null) {
    if (upload.value == null) {
      onUploading();
      upload.value = await _bridge.upload();
    }
    final blob = upload.value!;
    final url = blob['url'] as String;
    tags.add([
      'imeta',
      'url $url',
      'm ${blob['type']}',
      'x ${blob['sha256']}',
      'size ${blob['size']}',
      if (blob['dim'] case final String dim) 'dim $dim',
      if (blob['blurhash'] case final String blurhash) 'blurhash $blurhash',
    ]);
    content = content.isEmpty ? '![image]($url)' : '$content\n![image]($url)';
  }
  if (content.isEmpty) throw StateError('Add a note to send.');
  await _bridge.publish(content: content, tags: tags, createdAt: createdAt);
}
