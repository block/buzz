import 'dart:convert';

import '../crypto/event_signing.dart';
import '../crypto/nip44.dart';
import '../relay/nostr_models.dart';
import 'me_journal.dart';
import 'me_sync.dart';

/// A reviewed fixed audience and one signed link message. Retries reuse both,
/// so a changed destination roster cannot silently widen a partially sent share.
class MeChatShare {
  final String source;
  final String destination;
  final String destinationName;
  final Map<String, String> audience;
  final NostrEvent message;
  final String _scope;
  final String _record;

  MeChatShare._(
    this.source,
    this.destination,
    this.destinationName,
    this.audience,
    this.message,
    this._scope,
    this._record,
  );
  static String _key(MeSync sync, String source) =>
      'me-chat-share:${sync.community}:${sync.pubkey}:$source';

  factory MeChatShare.create(
    MeSync sync, {
    required String source,
    required String name,
    required String destination,
    required String destinationName,
    required Map<String, String> audience,
    bool forum = false,
  }) {
    if (audience.length > 100) {
      throw StateError('Share with at most 100 people or agents at a time.');
    }
    final label = name
        .replaceAll(RegExp(r'\s+'), ' ')
        .replaceAllMapped(
          RegExp(r'[\\`\[\]*_~<>&|]'),
          (match) => '\\${match[0]}',
        );
    final message = NostrEvent.fromJson(
      signNostrEvent(
        kind: forum ? EventKind.forumPost : EventKind.streamMessage,
        content:
            '[Chat: $label](buzz://channel/${Uri.encodeComponent(source)})',
        secretKey: sync.privateHex,
        tags: [
          ['h', destination],
        ],
      ).toMap(),
    );
    final record = nip44Encrypt(
      sync.key,
      jsonEncode({
        'source': source,
        'destination': destination,
        'destinationName': destinationName,
        'audience': audience,
        'message': message.toJson(),
      }),
    );
    return MeChatShare._(
      source,
      destination,
      destinationName,
      Map.unmodifiable(audience),
      message,
      _key(sync, source),
      record,
    );
  }

  /// Restore without issuing invitations or sending a message.
  static MeChatShare? pending(MeSync sync, String source) {
    final record = sync.prefs?.getString(_key(sync, source));
    if (record == null) return null;
    final data = jsonDecode(nip44Decrypt(sync.key, record)) as Map;
    final message = NostrEvent.fromJson(
      Map<String, dynamic>.from(data['message'] as Map),
    );
    verifiedNostrEventFromJson(jsonEncode(message.toJson()));
    final destination = data['destination'] as String;
    final audience = Map<String, String>.from(data['audience'] as Map);
    if (data['source'] != source ||
        message.pubkey != sync.pubkey ||
        message.channelId != destination ||
        audience.length > 100 ||
        audience.values.any((role) => role != 'member' && role != 'bot')) {
      throw const FormatException('The unfinished share is invalid.');
    }
    return MeChatShare._(
      source,
      destination,
      data['destinationName'] as String,
      Map.unmodifiable(audience),
      message,
      _key(sync, source),
      record,
    );
  }

  void _check(MeSync sync) {
    if (sync.disposed || _key(sync, source) != _scope) {
      throw StateError(
        'Your account or community changed. Open sharing again.',
      );
    }
  }

  /// Persist before the first grant, then retain this exact message through a
  /// lost OK response. Relay deduplication makes an explicit retry idempotent.
  Future<void> complete(
    MeSync sync, {
    required Future<void> Function(String, String) ensureRecipient,
    required Future<void> Function(NostrEvent) send,
  }) async {
    _check(sync);
    final prefs = sync.prefs;
    if (prefs == null) {
      throw StateError('Could not save sharing on this phone. Try again.');
    }
    final previous = prefs.getString(_scope);
    if (previous != null && previous != _record) {
      throw StateError('Finish the pending share first.');
    }
    if (!await prefs.setString(_scope, _record)) {
      throw StateError('Could not save sharing on this phone. Try again.');
    }
    _check(sync);
    for (final recipient in audience.entries) {
      await ensureRecipient(recipient.key, recipient.value);
      _check(sync);
    }
    await send(message);
    _check(sync);
    await clearMeJournal(
      prefs,
      _scope,
      _record,
      checkScope: () => _check(sync),
    );
  }
}
