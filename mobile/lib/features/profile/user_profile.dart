import 'package:flutter/foundation.dart';

import '../../shared/utils/string_utils.dart';

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

  /// Display name, or the canonical compact npub identity.
  String get label => displayName ?? shortPubkey(pubkey);

  /// First name letter or a varying character from the canonical npub.
  String get initial => displayName?.isNotEmpty == true
      ? displayName![0].toUpperCase()
      : pubkeyAvatarInitial(pubkey);
}

/// Optional profile handle shown beside a message author's display name.
String? messageUsernameLabel(UserProfile? profile) {
  final handle = profile?.nip05Handle?.trim();
  if (handle != null && handle.isNotEmpty) return handle;
  return null;
}
