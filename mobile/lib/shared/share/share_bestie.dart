import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../community/community.dart';

const _appGroupChannel = MethodChannel('buzz/app_group');

/// Tells the iOS share extension who [pubkey]'s Bestie is in [community], so
/// its picker can pin the Bestie DM. The extension can't decrypt the Bestie
/// preference.
Future<void> publishShareBestie({
  required String pubkey,
  required String community,
  required String? bestie,
  required String? name,
}) async {
  if (!Platform.isIOS) return;
  try {
    final path = await _appGroupChannel.invokeMethod<String>('cachesDirectory');
    if (path == null) return;
    final file = File('$path/share-bestie.json');
    if (bestie == null) {
      if (await file.exists()) await file.delete();
      return;
    }
    await file.parent.create(recursive: true);
    await file.writeAsString(
      jsonEncode({
        'pubkey': pubkey,
        'community': community,
        'bestie': bestie,
        'name': ?name,
      }),
      flush: true,
    );
  } on Object catch (error) {
    debugPrint('[ShareBestie] could not publish: $error');
  }
}

/// Hands the share extension [pubkey]'s conversations, a JSON list the app
/// has already named and sorted, so its picker opens without relay queries.
Future<void> publishShareConversations({
  required String pubkey,
  required String community,
  required String conversations,
}) async {
  if (!Platform.isIOS) return;
  try {
    final path = await _appGroupChannel.invokeMethod<String>('cachesDirectory');
    if (path == null) return;
    final file = File('$path/share-conversations.json');
    await file.parent.create(recursive: true);
    await file.writeAsString(
      '{"pubkey":${jsonEncode(pubkey)},"community":${jsonEncode(community)},'
      '"conversations":$conversations}',
      flush: true,
    );
  } on Object catch (error) {
    debugPrint('[ShareConversations] could not publish: $error');
  }
}

/// Gives the iOS share extension every signed-in community and its signing
/// key, so it can send as you whether or not notifications are on.
Future<void> publishShareAccounts(List<Community> communities) async {
  if (!Platform.isIOS) return;
  final accounts = <Map<String, Object?>>[];
  final signingKeys = <String, String>{};
  for (final community in communities) {
    final nsec = community.nsec;
    if (nsec == null || nsec.isEmpty) continue;
    try {
      final decoded = nostr.Nip19.decode(payload: nsec);
      if (decoded.prefix != nostr.Nip19Prefix.nsec ||
          decoded.data.length != 64) {
        continue;
      }
      signingKeys[community.id] = decoded.data;
    } on Object {
      continue;
    }
    accounts.add({
      'id': community.id,
      'name': community.name,
      'relayUrl': community.relayUrl,
      'pubkey': community.pubkey,
    });
  }
  try {
    await _appGroupChannel.invokeMethod<void>('syncShareAccounts', {
      'communities': accounts,
      'signingKeys': signingKeys,
    });
  } on Object catch (error) {
    debugPrint('[ShareAccounts] could not publish: $error');
  }
}
