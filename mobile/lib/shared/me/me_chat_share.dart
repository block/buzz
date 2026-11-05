import 'dart:convert';

import '../crypto/event_signing.dart';
import '../crypto/nip44.dart';
import '../relay/nostr_models.dart';
import '../relay/relay_session_types.dart';
import 'me_journal.dart';
import 'me_sync.dart';

/// A reviewed channel link. Uncertain retries reuse the signed link; only a
/// definitive rejection permits a freshly signed replacement.
class MeChatShare {
  final String source;
  final String destination;
  final String destinationName;
  final NostrEvent message;
  final String _scope;
  final String _record;

  MeChatShare._(
    this.source,
    this.destination,
    this.destinationName,
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
    bool forum = false,
  }) {
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
        'message': message.toJson(),
      }),
    );
    return MeChatShare._(
      source,
      destination,
      destinationName,
      message,
      _key(sync, source),
      record,
    );
  }

  /// Restore without changing access or sending a message. Legacy recipient
  /// lists are ignored: resuming a share must never enroll those identities.
  static MeChatShare? pending(MeSync sync, String source) {
    final record = sync.prefs?.getString(_key(sync, source));
    if (record == null) return null;
    final data = jsonDecode(nip44Decrypt(sync.key, record)) as Map;
    final message = NostrEvent.fromJson(
      Map<String, dynamic>.from(data['message'] as Map),
    );
    verifiedNostrEventFromJson(jsonEncode(message.toJson()));
    final destination = data['destination'] as String;
    if (data['source'] != source ||
        message.pubkey != sync.pubkey ||
        message.channelId != destination) {
      throw const FormatException('The unfinished share is invalid.');
    }
    return MeChatShare._(
      source,
      destination,
      data['destinationName'] as String,
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

  /// Forget only this reviewed share, without revoking access or deleting links.
  Future<void> discard(MeSync sync) async {
    _check(sync);
    final prefs = sync.prefs;
    if (prefs == null) throw StateError('Local sharing is unavailable.');
    await clearMeJournal(
      prefs,
      _scope,
      _record,
      checkScope: () => _check(sync),
    );
  }

  Future<void> _replaceRejectedMessage(MeSync sync) async {
    final prefs = sync.prefs!;
    await prefs.reload();
    _check(sync);
    if (prefs.getString(_scope) != _record) {
      throw StateError('The pending share changed. Open sharing again.');
    }
    final fresh = NostrEvent.fromJson(
      signNostrEvent(
        kind: message.kind,
        content: message.content,
        tags: message.tags,
        secretKey: sync.privateHex,
      ).toMap(),
    );
    final record = nip44Encrypt(
      sync.key,
      jsonEncode({
        'source': source,
        'destination': destination,
        'destinationName': destinationName,
        'message': fresh.toJson(),
      }),
    );
    try {
      if (!await prefs.setString(_scope, record)) {
        throw StateError('Could not save the replacement link. Try again.');
      }
    } catch (_) {
      await prefs.reload();
      rethrow;
    }
    _check(sync);
  }

  /// Persist before opening community reading, then retain this message through a
  /// lost OK response. Relay deduplication makes an explicit retry idempotent.
  Future<void> complete(
    MeSync sync, {
    required Future<void> Function(void Function() checkPending) prepare,
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
    void checkPending() {
      _check(sync);
      if (prefs.getString(_scope) != _record) {
        throw StateError('The pending share changed. Open sharing again.');
      }
    }

    await prepare(checkPending);
    checkPending();
    try {
      await send(message);
    } on RelayEventRejected {
      // A negative OK proves this attempt was not stored. Persist first, then
      // let the user explicitly retry; never replace an uncertain outcome.
      await _replaceRejectedMessage(sync);
      rethrow;
    }
    _check(sync);
    await clearMeJournal(
      prefs,
      _scope,
      _record,
      checkScope: () => _check(sync),
    );
  }
}
