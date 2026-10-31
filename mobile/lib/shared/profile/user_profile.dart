import 'package:flutter/foundation.dart';

import '../utils/string_utils.dart';

@immutable
class UserProfile {
  final String pubkey;
  final String? displayName;
  final String? avatarUrl;
  final String? about;
  final String? nip05Handle;

  /// NIP-OA verified owner pubkey from the profile's `auth` tag; non-null
  /// means this identity is an agent (mirrors desktop's `ownerPubkey`).
  final String? ownerPubkey;

  bool get isAgent => ownerPubkey != null;

  const UserProfile({
    required this.pubkey,
    this.displayName,
    this.avatarUrl,
    this.about,
    this.nip05Handle,
    this.ownerPubkey,
  });

  factory UserProfile.fromJson(Map<String, dynamic> json) => UserProfile(
    pubkey: json['pubkey'] as String,
    displayName: json['display_name'] as String?,
    avatarUrl: json['avatar_url'] as String?,
    about: json['about'] as String?,
    nip05Handle: json['nip05_handle'] as String?,
  );

  /// Short label: display name, or the compact npub form of the public key.
  ///
  /// Blank display names (empty or whitespace-only) fall back to the compact
  /// npub too — relay profiles can carry them — so a valid identity never
  /// renders an empty label, like [initial]. Nonblank names render as
  /// authored: trim only tests blankness, so authored padding survives
  /// (unlike [initial], which reads the trimmed padding).
  String get label {
    final name = displayName;
    return name != null && name.trim().isNotEmpty ? name : shortPubkey(pubkey);
  }

  /// First letter for fallback avatar.
  String get initial {
    final name = displayName?.trim();
    if (name != null && name.isNotEmpty) return name[0].toUpperCase();
    // Hex-derived (not npub-derived) so unnamed identities keep distinct
    // initials instead of every npub rendering `N`.
    return pubkey.isNotEmpty ? pubkey[0].toUpperCase() : '?';
  }
}

/// Optional profile handle shown beside a message author's display name.
String? messageUsernameLabel(UserProfile? profile) {
  final handle = profile?.nip05Handle?.trim();
  if (handle != null && handle.isNotEmpty) return handle;
  return null;
}
