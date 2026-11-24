import 'dart:convert';

import 'package:flutter/services.dart';

import '../shared/relay/nostr_models.dart';
import 'share_conversations.dart';

/// The native side of the iOS share extension: what was shared, the signed-in
/// community, and relay requests signed in the extension's own process.
class ShareBridge {
  const ShareBridge([
    this._channel = const MethodChannel('buzz/share_extension'),
  ]);

  final MethodChannel _channel;

  Future<SharePayload> load() async {
    final raw = await _channel.invokeMapMethod<String, Object?>('load');
    return SharePayload.fromMap(raw ?? const {});
  }

  Future<List<NostrEvent>> query(List<NostrFilter> filters) async {
    final raw = await _channel.invokeListMethod<Object?>('query', {
      'filters': [for (final filter in filters) filter.toJson()],
    });
    return [
      for (final event in raw ?? const [])
        NostrEvent.fromJson(
          jsonDecode(jsonEncode(event)) as Map<String, dynamic>,
        ),
    ];
  }

  /// Uploads the shared photo and returns its blob descriptor.
  Future<Map<String, Object?>> upload() async =>
      (await _channel.invokeMapMethod<String, Object?>('upload')) ?? const {};

  /// Signs and submits a message. The same [createdAt] gives a retry the same
  /// event ID, so the relay drops a duplicate whose first answer was lost.
  Future<void> publish({
    int kind = EventKind.streamMessage,
    required String content,
    required List<List<String>> tags,
    required int createdAt,
  }) => _channel.invokeMethod('publish', {
    'kind': kind,
    'content': content,
    'tags': tags,
    'createdAt': createdAt,
  });

  Future<void> close() => _channel.invokeMethod('close');
  Future<void> cancel() => _channel.invokeMethod('cancel');
}

class SharePayload {
  const SharePayload({
    required this.text,
    required this.missed,
    this.imagePath,
    this.imageMime,
    this.filePath,
    this.filename,
    this.problem,
    this.suggested,
    this.community,
    this.signedOut,
    this.bestie,
    this.conversations,
  });

  factory SharePayload.fromMap(Map<String, Object?> map) {
    final image = map['image'] as Map<Object?, Object?>?;
    final file = map['file'] as Map<Object?, Object?>?;
    final community = map['community'] as Map<Object?, Object?>?;
    final bestie = map['bestie'] as Map<Object?, Object?>?;
    final conversations = map['conversations'] as List<Object?>?;
    return SharePayload(
      text: [for (final item in map['text'] as List? ?? const []) '$item'],
      missed: map['missed'] as int? ?? 0,
      imagePath: image?['path'] as String?,
      imageMime: image?['mime'] as String?,
      filePath: file?['path'] as String?,
      filename: file?['filename'] as String?,
      problem: map['problem'] as String?,
      suggested: _suggestedChannel(map['suggested'] as String?, community),
      community: community == null
          ? null
          : (
              name: community['name'] as String? ?? '',
              pubkey: community['pubkey'] as String? ?? '',
            ),
      signedOut: map['signedOut'] as String?,
      bestie: bestie == null
          ? null
          : (
              pubkey: bestie['pubkey'] as String? ?? '',
              name: bestie['name'] as String? ?? '',
            ),
      conversations: conversations == null
          ? null
          : [
              for (final item
                  in conversations.whereType<Map<Object?, Object?>>())
                if (item['id'] case final String id)
                  ShareConversation(
                    id: id,
                    name: item['name'] as String? ?? '',
                    isDm: item['dm'] == true,
                    isSolo: item['solo'] == true,
                    recipients: [
                      for (final p in item['recipients'] as List? ?? const [])
                        '$p',
                    ],
                    lastActivity: item['at'] as int? ?? 0,
                  ),
            ],
    );
  }

  final List<String> text;
  final int missed;
  final String? imagePath;
  final String? imageMime;

  /// A shared file other than a photo, sent as is.
  final String? filePath;
  final String? filename;
  final String? problem;

  /// The channel of the iOS share suggestion picked, when it belongs to this
  /// community.
  final String? suggested;
  final ({String name, String pubkey})? community;
  final String? signedOut;
  final ({String pubkey, String name})? bestie;

  /// The app's saved conversations, newest activity first; null when it
  /// hasn't saved any for this account.
  final List<ShareConversation>? conversations;

  /// The note's starting text: each link or text item on its own line.
  String get note => text.join('\n');
}

/// The suggestion's channel, from the `[communityId, channelId]` identifier
/// the app donates, when it names this community.
String? _suggestedChannel(
  String? identifier,
  Map<Object?, Object?>? community,
) {
  if (identifier == null || community == null) return null;
  try {
    final parts = jsonDecode(identifier) as List;
    if (parts.length == 2 && parts[0] == community['id']) {
      return parts[1] as String;
    }
  } on Object {
    return null;
  }
  return null;
}
