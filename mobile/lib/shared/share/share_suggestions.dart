import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../community/community_provider.dart';

import 'pending_share.dart';

const _channel = MethodChannel('buzz/share_suggestions');

/// Tells iOS the person just sent a top-level message in this conversation,
/// so the share sheet can suggest it. Thread replies and incoming messages
/// are never donated (see CONTEXT.md "Share target").
///
/// The donation only ranks suggestions, so a failure is logged rather than
/// surfaced: the message itself was sent.
Future<void> donateShareSuggestion({
  required String communityId,
  required String channelId,
  required String displayName,
  required int recipientCount,
}) async {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  try {
    await _channel.invokeMethod<void>('donate', {
      'conversationIdentifier': shareConversationIdentifier(
        communityId: communityId,
        channelId: channelId,
      ),
      'displayName': displayName,
      'recipientCount': recipientCount < 1 ? 1 : recipientCount,
    });
  } on MissingPluginException {
    // Flutter tests and non-Runner embeddings do not install the native bridge.
  } on PlatformException catch (error) {
    debugPrint('share-suggestions: donation failed: ${error.code}');
  }
}

/// Donates a conversation in the active community after a top-level send.
void donateActiveShareSuggestion(
  WidgetRef ref, {
  required String channelId,
  required String displayName,
  required int recipientCount,
}) {
  if (defaultTargetPlatform != TargetPlatform.iOS) return;
  // Sending means the signed-in app already resolved its active community;
  // never start loading it just to donate.
  final community = ref.exists(activeCommunityProvider)
      ? ref.read(activeCommunityProvider).asData?.value
      : null;
  if (community == null) {
    debugPrint('share-suggestions: no active community after a send');
    return;
  }
  unawaited(
    donateShareSuggestion(
      communityId: community.id,
      channelId: channelId,
      displayName: displayName,
      recipientCount: recipientCount,
    ),
  );
}
