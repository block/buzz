import 'package:flutter/foundation.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../../shared/crypto/nip44.dart';
import '../sidebar_flag_sync.dart';
import 'channel_mutes_storage.dart';

class ChannelMutesCrypto {
  final Uint8List _conversationKey;

  ChannelMutesCrypto(String nsec, String pubkey)
    : _conversationKey = _deriveKey(nsec, pubkey);

  static Uint8List _deriveKey(String nsec, String pubkey) {
    final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
    return getConversationKey(privkeyHex, pubkey);
  }

  String encrypt(String plaintext) => nip44Encrypt(_conversationKey, plaintext);

  String decrypt(String ciphertext) =>
      nip44Decrypt(_conversationKey, ciphertext);
}

/// Syncs the account's channel mutes.
class ChannelMutesManager extends SidebarFlagSync<ChannelMuteEntry> {
  ChannelMutesManager({
    required super.pubkey,
    required super.relayUrl,
    required super.prefs,
    required ChannelMutesCrypto crypto,
    required super.relaySession,
    required super.signedEventRelay,
    required super.remoteEnabled,
    required super.onChanged,
    @visibleForTesting super.startupRetryBaseDelay = const Duration(seconds: 2),
  }) : super(
         coordinate: 'channel-mutes',
         cacheKey: channelMutesKey(pubkey, relayUrl),
         cachedFlags: ChannelMutesStorage(
           prefs,
         ).read(pubkey, relayUrl).channels,
         decode: (json, strict) =>
             ChannelMuteStore.fromJson(json, strict: strict).channels,
         encode: (flags) => ChannelMuteStore(channels: flags).toJson(),
         entry: (enabled, timestamp) =>
             ChannelMuteEntry(muted: enabled, updatedAt: timestamp),
         decrypt: crypto.decrypt,
         encrypt: crypto.encrypt,
       );

  ChannelMuteStore get store => ChannelMuteStore(channels: values);

  void muteChannel(String channelId) => setFlag(channelId, true);

  void unmuteChannel(String channelId) => setFlag(channelId, false);
}
