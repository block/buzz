import 'package:flutter/foundation.dart';
import 'package:unorm_dart/unorm_dart.dart' as unorm;

import '../relay/nostr_models.dart';

/// NIP-FI verified profile projection: a NIP-85 user assertion (kind 30382)
/// signed by the community relay's NIP-11 `self` key.
const verifiedProfileKind = 30382;

/// Subjects per subscription. Keeps REQs inside relay filter and message limits.
const verifiedProfileBatchSize = 128;

final RegExp _lowerHex64 = RegExp(r'^[0-9a-f]{64}$');
final RegExp _lowerHex128 = RegExp(r'^[0-9a-f]{128}$');
final RegExp _username = RegExp(r'^[a-z0-9._-]{1,64}$');
final RegExp _decimal = RegExp(r'^[0-9]+$');
final RegExp _prohibited = RegExp(
  r'[\p{Cc}\p{Cf}\p{Cs}\p{Co}\p{Cn}\p{Zl}\p{Zp}]',
  unicode: true,
);
final RegExp _edgeWhitespace = RegExp(
  r'^\p{White_Space}|\p{White_Space}$',
  unicode: true,
);

/// The community key against which projections are verified, as advertised by
/// a successful NIP-11 discovery that enables `public_profile`.
@immutable
class VerifiedProfileAuthority {
  const VerifiedProfileAuthority({
    required this.pubkey,
    required this.multipleIssuers,
  });

  final String pubkey;
  final bool multipleIssuers;

  @override
  bool operator ==(Object other) =>
      other is VerifiedProfileAuthority &&
      other.pubkey == pubkey &&
      other.multipleIssuers == multipleIssuers;

  @override
  int get hashCode => Object.hash(pubkey, multipleIssuers);
}

/// The outcome of a *successful* discovery response. A failed or malformed
/// response is not an outcome: it throws and must not replace prior state.
sealed class VerifiedProfileDiscovery {
  const VerifiedProfileDiscovery();
}

/// `federated_identity.public_profile` is present, valid and bound to `self`.
class VerifiedProfilesEnabled extends VerifiedProfileDiscovery {
  const VerifiedProfilesEnabled(this.authority);
  final VerifiedProfileAuthority authority;
}

/// A valid response omits `federated_identity` or `public_profile`: an
/// explicit operator disable that hides every mark, including cached ones.
class VerifiedProfilesDisabled extends VerifiedProfileDiscovery {
  const VerifiedProfilesDisabled();
}

/// Parses a decoded NIP-11 document.
///
/// Throws [FormatException] for a malformed document, which callers treat as
/// a failed discovery (the previous state stands).
VerifiedProfileDiscovery parseVerifiedProfileDiscovery(Object? info) {
  if (info is! Map) throw const FormatException('NIP-11 is not an object');
  final federated = info['federated_identity'];
  if (federated == null && !info.containsKey('federated_identity')) {
    return const VerifiedProfilesDisabled();
  }
  if (federated is! Map) {
    throw const FormatException('Malformed federated_identity');
  }
  if (!federated.containsKey('public_profile')) {
    return const VerifiedProfilesDisabled();
  }
  final profile = federated['public_profile'];
  final self = info['self'];
  if (profile is! Map ||
      self is! String ||
      !_lowerHex64.hasMatch(self) ||
      profile['kind'] != verifiedProfileKind ||
      profile['multiple_issuers'] is! bool) {
    throw const FormatException('Malformed public_profile discovery');
  }
  return VerifiedProfilesEnabled(
    VerifiedProfileAuthority(
      pubkey: self,
      multipleIssuers: profile['multiple_issuers'] as bool,
    ),
  );
}

/// A name, username-free label, or issuer label: 1 to 64 scalar values in NFC,
/// no `Cc`, `Cf`, `Cs`, `Co`, `Cn`, `Zl` or `Zp` characters, and no
/// `White_Space` at either end.
bool isValidVerifiedName(String value) {
  final length = value.runes.length;
  return length >= 1 &&
      length <= 64 &&
      unorm.nfc(value) == value &&
      !_prohibited.hasMatch(value) &&
      !_edgeWhitespace.hasMatch(value);
}

/// A verified name as the relay last published it. Historical: it records the
/// issuer's name-to-key assertion at [verifiedAt], never current access.
@immutable
class VerifiedProfile {
  const VerifiedProfile({
    required this.displayName,
    required this.username,
    required this.issuerLabel,
    required this.verifiedAt,
    required this.checked,
    required this.multipleIssuers,
    this.fullName,
  });

  final String displayName;
  final String username;
  final String? fullName;
  final String issuerLabel;

  /// Unix seconds of the issuer assertion the relay verified.
  final int verifiedAt;

  /// True only when reconciled with the relay on the current connection and a
  /// fresh discovery; otherwise this is a cached record.
  final bool checked;

  final bool multipleIssuers;

  VerifiedProfile withChecked(bool value) => value == checked
      ? this
      : VerifiedProfile(
          displayName: displayName,
          username: username,
          fullName: fullName,
          issuerLabel: issuerLabel,
          verifiedAt: verifiedAt,
          checked: value,
          multipleIssuers: multipleIssuers,
        );

  @override
  bool operator ==(Object other) =>
      other is VerifiedProfile &&
      other.displayName == displayName &&
      other.username == username &&
      other.fullName == fullName &&
      other.issuerLabel == issuerLabel &&
      other.verifiedAt == verifiedAt &&
      other.checked == checked &&
      other.multipleIssuers == multipleIssuers;

  @override
  int get hashCode => Object.hash(
    displayName,
    username,
    fullName,
    issuerLabel,
    verifiedAt,
    checked,
    multipleIssuers,
  );
}

/// The total size, in UTF-16 code units of content and every tag element
/// (plus one per tag and per element), above which an event is dropped before any buffering or proof work. A valid
/// projection is at most about 600 (eight two-element tags of at most 128), so
/// a small malformed event still takes its address, while a full pre-EOSE
/// buffer stays near 2 MB.
const _maxEventSize = 4096;

int? _boundedSize(NostrEvent event) {
  var size = event.content.length;
  if (size > _maxEventSize) return null;
  for (final tag in event.tags) {
    // Each tag and element counts at least one, so empty ones are bounded too.
    size += 1;
    for (final value in tag) {
      size += value.length + 1;
      if (size > _maxEventSize) return null;
    }
    if (size > _maxEventSize) return null;
  }
  return size;
}

/// The subject a kind 30382 event addresses: its first `d` tag, when the
/// event is by [authority], within [_maxEventSize], and the tag is a
/// lowercase hex pubkey.
///
/// Projection shape is not checked here: a newer signed event with a
/// malformed profile still replaces the older one at this address, so it
/// clears the earlier verified name instead of preserving it (only the latest
/// projection counts). Callers must have verified id and signature.
String? verifiedProfileSubject(
  NostrEvent event,
  VerifiedProfileAuthority authority,
) {
  if (event.kind != verifiedProfileKind ||
      event.pubkey != authority.pubkey ||
      // Metadata is bounded too: an id or signature is fixed-size hex.
      !_lowerHex64.hasMatch(event.id) ||
      !_lowerHex128.hasMatch(event.sig) ||
      _boundedSize(event) == null) {
    return null;
  }
  for (final tag in event.tags) {
    if (tag.isNotEmpty && tag.first == 'd') {
      return tag.length > 1 && _lowerHex64.hasMatch(tag[1]) ? tag[1] : null;
    }
  }
  return null;
}

/// NIP-01 replaceable ordering: the later `created_at` wins, and a tie goes to
/// the lower event id.
bool supersedes(NostrEvent candidate, NostrEvent? current) =>
    current == null ||
    candidate.createdAt > current.createdAt ||
    (candidate.createdAt == current.createdAt &&
        candidate.id.compareTo(current.id) < 0);

/// Strictly validates the projection carried by [event] (whose id and
/// signature the caller has already verified).
///
/// Returns null unless the event is the shape NIP-FI defines: empty content;
/// exactly seven tags (eight with `full_name`), each of two elements with no
/// repeated name; `d` and `p` equal; `active` equal to `true`; well-formed
/// names, username and issuer label; and a `verified_at` no later than
/// `created_at`. A retraction (`active=false`) is therefore null.
VerifiedProfile? parseVerifiedProfile(
  NostrEvent event,
  VerifiedProfileAuthority authority, {
  required bool checked,
}) {
  final subject = verifiedProfileSubject(event, authority);
  if (subject == null ||
      event.content != '' ||
      (event.tags.length != 7 && event.tags.length != 8)) {
    return null;
  }
  final tags = <String, String>{};
  for (final tag in event.tags) {
    if (tag.length != 2 || tags.containsKey(tag[0])) return null;
    tags[tag[0]] = tag[1];
  }
  const required = {
    'd',
    'p',
    'active',
    'display_name',
    'username',
    'issuer_label',
    'verified_at',
  };
  if (!required.every(tags.containsKey) ||
      tags.length !=
          required.length + (tags.containsKey('full_name') ? 1 : 0)) {
    return null;
  }
  final fullName = tags['full_name'];
  final displayName = tags['display_name']!;
  final username = tags['username']!;
  final issuerLabel = tags['issuer_label']!;
  final rawVerifiedAt = tags['verified_at']!;
  if (tags['p'] != subject ||
      tags['active'] != 'true' ||
      (fullName != null && !isValidVerifiedName(fullName)) ||
      !isValidVerifiedName(displayName) ||
      !_username.hasMatch(username) ||
      !isValidVerifiedName(issuerLabel) ||
      !_decimal.hasMatch(rawVerifiedAt)) {
    return null;
  }
  final verifiedAt = int.tryParse(rawVerifiedAt);
  if (verifiedAt == null || verifiedAt > event.createdAt) return null;
  return VerifiedProfile(
    displayName: displayName,
    username: username,
    fullName: fullName,
    issuerLabel: issuerLabel,
    verifiedAt: verifiedAt,
    checked: checked,
    multipleIssuers: authority.multipleIssuers,
  );
}
