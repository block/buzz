import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/auth/auth_provider.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/me/me_assistants.dart';
import '../../shared/drafts/compose_drafts_provider.dart';
import '../../shared/mentions/mention_tags.dart';
import '../../shared/outbox/text_outbox_delivery.dart';
import '../../shared/outbox/text_outbox_store.dart';
import '../../shared/relay/relay.dart';
import '../../shared/share/share_suggestions.dart';
import 'channel.dart';
import 'channel_management_provider.dart';
import 'channels_provider.dart';
import 'message_mention_pubkeys.dart';
import 'send_message_provider.dart';

/// Whether this client has the native backup-excluded outbox directory.
final textOutboxSupportedProvider = Provider<bool>(
  (_) => Platform.isIOS || Platform.isAndroid,
);

/// The active saved membership authorized to enqueue and deliver text.
final textOutboxScopeProvider = Provider<TextOutboxScope?>((ref) {
  final active = ref.watch(activeCommunityProvider).asData?.value;
  final config = ref.watch(relayConfigProvider);
  final pubkey = ref.watch(myPubkeyProvider);
  if (active == null ||
      pubkey == null ||
      !config.isSameCommunity(
        RelayConfig(baseUrl: active.relayUrl, nsec: active.nsec),
      )) {
    return null;
  }
  final communities = ref.watch(communityListProvider).asData?.value;
  if (communities?.any(
        (item) => item.id == active.id && item.nsec == active.nsec,
      ) !=
      true) {
    return null;
  }
  return (communityId: active.id, pubkey: pubkey);
});

final _outboxChangesProvider = StreamProvider.autoDispose<int>((ref) {
  return ref.watch(textOutboxStoreProvider).changes;
});

/// The scope's recoverable text, updated after each durable change.
final textOutboxMessagesProvider = FutureProvider.autoDispose
    .family<List<TextOutboxMessage>, TextOutboxScope>((ref, scope) {
      ref.watch(_outboxChangesProvider);
      return ref.read(textOutboxStoreProvider).list(scope);
    });

/// Enqueues the settled composer payload and checks that it still belongs to
/// the active community. A local write failure propagates to the composer.
Future<void> queueConversationText(
  WidgetRef ref, {
  required TextOutboxScope? scope,
  required String id,
  required String channelId,
  required Channel? channel,
  required String text,
  required String content,
  required List<String> recipients,
  required List<List<String>> tags,
  String? parentId,
  String? rootId,
  bool personal = false,
  String? implicitSoloAgent,
}) async {
  if (scope == null || ref.read(textOutboxScopeProvider) != scope) {
    throw ActiveCommunityChangedError('Queued send changed communities.');
  }
  final resolved = channel?.isDm == true
      ? messageMentionPubkeys(
          channel: channel!,
          senderPubkey: scope.pubkey,
          explicitMentions: recipients,
          dmRecipientPubkeys: dmParticipantPubkeys(
            channel,
            ref.read(channelMembersProvider(channelId)).value,
          ),
        )
      : recipients;
  try {
    await ref.read(textOutboxStoreProvider).enqueue(scope, id, (
      channelId: channelId,
      parentId: parentId,
      rootId: rootId,
      text: text,
      content: content,
      recipients: resolved,
      tags: tags,
      implicitSoloAgent: implicitSoloAgent,
      personal: personal,
    ));
  } catch (error) {
    debugPrint('Could not commit a queued send: ${error.runtimeType}');
    throw StateError('Couldn’t save this message on your device. Try again.');
  }
}

/// Runs the outbox across conversations while the signed-in app is active.
final textOutboxRunnerProvider = Provider<void>((ref) {
  final store = ref.watch(textOutboxStoreProvider);
  final session = ref.read(relaySessionProvider.notifier);
  var epoch = 0;
  var running = false;
  var requested = false;
  var disposed = false;
  var retrySeconds = 2;
  Timer? retry;
  TextOutboxScope? activeScope() =>
      disposed ||
          ref.read(authProvider).asData?.value.status !=
              AuthStatus.authenticated
      ? null
      : ref.read(textOutboxScopeProvider);

  Future<void> drain() async {
    if (running || disposed || !requested) return;
    requested = false;
    final scope = activeScope();
    if (scope == null ||
        store.isRetiring(scope.communityId) ||
        ref.read(relaySessionProvider).status != SessionStatus.connected) {
      return;
    }
    final startEpoch = epoch;
    bool isCurrent() =>
        !disposed &&
        startEpoch == epoch &&
        activeScope() == scope &&
        !store.isRetiring(scope.communityId) &&
        ref.read(relaySessionProvider).status == SessionStatus.connected;
    running = true;
    var needsRetry = false;
    try {
      final delivery = TextOutboxDelivery(
        store: store,
        releaseDraft: (message) => ref
            .read(composeDraftsProvider.notifier)
            .releaseOutboxDraft(
              composeDraftKey(
                message.draft.channelId,
                threadHeadId: message.draft.parentId,
              ),
              message.id,
            ),
        prepare: (message, nowMilliseconds) async {
          final draft = message.draft;
          if (draft.implicitSoloAgent case final agent?) {
            ref.invalidate(meAssistantOwnershipProvider(agent));
            final owned = await ref.read(
              meAssistantOwnershipProvider(agent).future,
            );
            if (!isCurrent()) {
              throw ActiveCommunityChangedError(
                'Queued send changed communities.',
              );
            }
            if (!owned) {
              throw const MentionTagException(MentionTagError.notMember);
            }
          }
          final sender = ref.read(
            draft.personal ? meSendMessageProvider : sendMessageProvider,
          );
          return sender.signQueued(
            id: message.id,
            channelId: draft.channelId,
            content: draft.content,
            parentEventId: draft.parentId,
            rootEventId: draft.rootId,
            recipients: draft.recipients,
            tags: draft.tags,
            nowMilliseconds: nowMilliseconds,
            isCurrent: isCurrent,
          );
        },
        publish: session.publishQueued,
        lookup: (event) async {
          final events = await session.queryRelay([
            NostrFilter(
              ids: [event.id],
              kinds: const [EventKind.streamMessage],
              limit: 1,
              extensions: const {'consistency': 'strong'},
            ),
          ]);
          return events.any(
            (match) =>
                match.id == event.id &&
                match.sig == event.sig &&
                match.pubkey == event.pubkey,
          );
        },
        showAccepted: (message, event) {
          final draft = message.draft;
          ref
              .read(
                draft.personal ? meSendMessageProvider : sendMessageProvider,
              )
              .showQueuedAccepted(draft.channelId, event);
          if (draft.parentId == null &&
              defaultTargetPlatform == TargetPlatform.iOS) {
            final channel = ref
                .read(channelsProvider)
                .value
                ?.where((item) => item.id == draft.channelId)
                .firstOrNull;
            if (channel != null) {
              unawaited(
                donateShareSuggestion(
                  communityId: scope.communityId,
                  channelId: channel.id,
                  displayName: channel.displayLabel(
                    currentPubkey: scope.pubkey,
                  ),
                  recipientCount: channel.memberCount - 1,
                ),
              );
            }
          }
        },
      );
      needsRetry = await delivery.drain(scope, isCurrent);
    } catch (error) {
      debugPrint('Message outbox unavailable: ${error.runtimeType}');
      needsRetry = true;
    } finally {
      running = false;
    }
    if (disposed) return;
    if (needsRetry && isCurrent()) {
      retry?.cancel();
      retry = Timer(Duration(seconds: retrySeconds), () {
        requested = true;
        unawaited(drain());
      });
      retrySeconds = (retrySeconds * 2).clamp(2, 60);
    } else {
      retrySeconds = 2;
      if (requested) unawaited(drain());
    }
  }

  void wake() {
    retry?.cancel();
    requested = true;
    unawaited(drain());
  }

  final subscription = store.changes.listen((_) => wake());
  ref.listen(authProvider.select((auth) => auth.asData?.value.status), (_, _) {
    epoch++;
    wake();
  });
  ref.listen(textOutboxScopeProvider, (previous, next) {
    if (previous != next) epoch++;
    wake();
  });
  ref.listen(relaySessionProvider.select((session) => session.status), (_, _) {
    epoch++;
    wake();
  });
  ref.onDispose(() {
    disposed = true;
    retry?.cancel();
    unawaited(subscription.cancel());
  });
  scheduleMicrotask(() {
    if (!disposed) wake();
  });
});
