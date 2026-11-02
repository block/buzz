import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/verified_profiles/verified_profile.dart';
import 'package:nostr/nostr.dart' as nostr;

final relayKeys = nostr.Keys.generate();
final authority = VerifiedProfileAuthority(
  pubkey: relayKeys.public,
  multipleIssuers: false,
);
final subjectA = 'a' * 64;
final subjectB = 'b' * 64;

List<List<String>> profileTags(
  String subject, {
  String name = 'Ada Lovelace',
  String username = 'ada',
  String issuer = 'Example Corp',
  String verifiedAt = '1000',
  String? fullName,
}) => [
  ['d', subject],
  ['p', subject],
  ['active', 'true'],
  ['display_name', name],
  ['username', username],
  ['issuer_label', issuer],
  ['verified_at', verifiedAt],
  if (fullName != null) ['full_name', fullName],
];

List<List<String>> retractionTags(String subject) => [
  ['d', subject],
  ['p', subject],
  ['active', 'false'],
];

/// A really signed kind 30382 event by [author] (default: the relay key).
NostrEvent signedProjection(
  List<List<String>> tags, {
  int createdAt = 2000,
  String content = '',
  nostr.Keys? author,
}) {
  final key = author ?? relayKeys;
  final event = signNostrEvent(
    kind: verifiedProfileKind,
    content: content,
    secretKey: key.secret,
    createdAt: createdAt,
    tags: tags,
  );
  return NostrEvent.fromJson(event.toMap());
}

nostr.Keys relayKeysOther() => nostr.Keys.generate();
