import 'dart:async';

import '../../shared/me/hold_to_talk.dart';
import '../../shared/relay/relay.dart';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/agents/owned_agent_directory.dart';
import '../../shared/agents/remote_agents.dart';
import '../../shared/me/default_agent.dart';
import '../../shared/animated_avatar.dart';
import '../../shared/me/me_sync.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/agent_navigation_icon.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/preset_agent_avatar.dart';
import '../../shared/widgets/buzz_callout.dart';
import 'first_agent_callout.dart';
import 'empty_bestie_avatar.dart';
import 'bestie_conversation.dart';
import 'channel.dart';
import 'default_agent_voice_control.dart';
import 'default_agent_voice_draft.dart';
import 'voice_note_recording.dart';
import 'send_message_provider.dart';
import 'channels_provider.dart';
import 'channel_management_provider.dart';

/// Persistent chat and voice shortcut to the agent chosen in Agents.
class DefaultAgentLauncher extends HookConsumerWidget {
  const DefaultAgentLauncher({required this.onChooseAgent, super.key});
  final VoidCallback onChooseAgent;
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final preference = ref.watch(defaultAgentProvider);
    final holdToTalk = ref.watch(holdToTalkProvider);
    final scope = ref.watch(meSyncProvider);
    final scopeIdentity =
        '${scope.community}:${ref.watch(myPubkeyProvider) ?? 'signed-out'}';
    final savedRecipients = ref.watch(
      pendingAgentVoiceRecipientsProvider(scopeIdentity),
    );
    final savedRecipient = savedRecipients.value?.firstOrNull;
    final selected = useMemoized(() => <String?>[null], [scopeIdentity]);
    if (!preference.isLoading && !preference.hasError) {
      selected[0] = preference.value;
    }
    final pubkey = savedRecipient ?? selected[0];
    final profiles = ref.watch(userCacheProvider);
    final profile = profiles[selected[0]];
    final opening = useState(false);
    final resolving = useState(false);
    final remoteAgents = ref.watch(remoteAgentsEnabledProvider);
    // Only someone with no agents at all is invited to make a first one.
    // Cheap checks come first, so the rest are watched only when relevant.
    final showCallout =
        remoteAgents &&
        pubkey == null &&
        !preference.isLoading &&
        !preference.hasError &&
        !ref.watch(firstAgentCalloutSeenProvider) &&
        ref
                .watch(ownedAgentDirectoryProvider)
                .unwrapPrevious()
                .value
                ?.isEmpty ==
            true;
    useEffect(() {
      if (pubkey != null) {
        ref.read(userCacheProvider.notifier).preload([pubkey]);
      }
      // Having a Bestie, however it was chosen, retires the tip for good. A
      // failed read can still hold the previous community's Bestie, so only
      // current data counts.
      if (pubkey != null &&
          !preference.hasError &&
          remoteAgents &&
          !ref.read(firstAgentCalloutSeenProvider)) {
        dismissFirstAgentCallout(ref);
      }
      return null;
    }, [pubkey, preference.hasError, remoteAgents]);
    final draftIdentity = '$scopeIdentity:$pubkey';
    final label = selected[0] != null
        ? 'Chat with ${profile?.label ?? 'your Bestie'}'
        : preference.hasError
        ? 'Retry Bestie'
        : 'Bestie';
    Future<void> open({VoiceNoteRecording? recording}) async {
      final destination = recording == null ? selected[0] : pubkey;
      bool destinationIsCurrent() {
        if (ref.read(meSyncProvider) != scope) return false;
        // An explicit retry keeps the recipient captured with the draft.
        if (recording != null) return true;
        final current = ref.read(defaultAgentProvider);
        return current.hasError ||
            (!current.isLoading && current.value == destination);
      }

      if (opening.value || !destinationIsCurrent()) {
        if (recording != null) throw StateError('The agent is unavailable.');
        return;
      }
      unawaited(HapticFeedback.lightImpact());
      if (preference.hasError && recording == null) {
        ref.invalidate(defaultAgentProvider);
        return;
      }
      if (destination == null) {
        if (remoteAgents) dismissFirstAgentCallout(ref);
        if (recording != null) throw StateError('No default agent selected.');
        onChooseAgent();
        return;
      }
      if (recording == null) opening.value = true;
      try {
        if (recording == null) {
          await openBestieConversation(
            context,
            ref,
            destination,
            onResolving: (value) => resolving.value = value,
          );
          return;
        }
        final channelState = ref.read(channelsProvider);
        // AsyncValue may retain the previous community's list during reload.
        final channels = channelState.isLoading || channelState.hasError
            ? const <Channel>[]
            : channelState.value ?? const <Channel>[];
        final notifier = ref.read(channelsProvider.notifier);
        final cached = channels
            .where(
              (channel) => isActiveDirectConversation(
                channel,
                notifier.cachedMembersForChannel(channel.id),
                ref.read(myPubkeyProvider),
                destination,
              ),
            )
            .firstOrNull;
        final draft = ref.read(defaultAgentVoiceDraftProvider(draftIdentity));
        final uploadService = ref.read(mediaUploadServiceProvider);
        // Resolve the destination while packaging/uploading, with both errors
        // observed even if one operation fails before the other completes.
        final (channel, prepared) = await (
          cached != null
              ? Future.value(cached)
              : ref.read(channelActionsProvider).openDm(pubkeys: [destination]),
          () async {
            final event = await draft.readEvent();
            if (event != null) return (event, null);
            final descriptor = await uploadService.uploadVoiceNote(
              recording.file,
              duration: recording.duration,
            );
            return (null, descriptor);
          }(),
        ).wait;
        if (context.mounted) resolving.value = false;
        if (!context.mounted) throw StateError('The conversation was closed.');
        // A community switch invalidates an in-flight destination.
        if (!destinationIsCurrent()) {
          throw StateError('The agent changed.');
        }
        final session = ref.read(relaySessionProvider.notifier);
        final previousEvent = prepared.$1;
        if (previousEvent != null) {
          if (!context.mounted || !destinationIsCurrent()) {
            throw StateError('The community changed.');
          }
          await draft.retryEvent(
            session: session,
            signer: SignedEventRelay(
              session: session,
              nsec: ref.read(relayConfigProvider).nsec,
            ),
            generation: recording.draftGeneration,
            validate: () {
              if (!context.mounted || !destinationIsCurrent()) {
                throw StateError('The community changed.');
              }
            },
          );
        } else {
          final sender = ref.read(sendMessageProvider);
          final descriptor = prepared.$2!;
          if (!context.mounted || !destinationIsCurrent()) {
            throw StateError('The agent or community changed.');
          }
          await sender.call(
            channelId: channel.id,
            channel: channel,
            content: descriptor.toMarkdownImage(),
            mentionPubkeys: [destination],
            mediaTags: [descriptor.toImetaTag()],
            beforePublish: (event) async {
              if (!context.mounted || !destinationIsCurrent()) {
                throw StateError('The agent or community changed.');
              }
              await draft.saveEvent(
                event,
                generation: recording.draftGeneration,
              );
            },
          );
        }
        if (!context.mounted || !destinationIsCurrent()) {
          return;
        }
        unawaited(openBestieDestination(context, channel));
      } finally {
        if (context.mounted && recording == null) opening.value = false;
      }
    }

    final avatar = ExcludeSemantics(
      child: resolving.value
          ? const SizedBox.square(
              dimension: 22,
              child: BuzzLoadingIndicator(
                size: 22,
                semanticLabel: 'Opening conversation',
              ),
            )
          : selected[0] == null
          ? preference.isLoading || preference.hasError
                ? const AgentNavigationIcon(size: 24)
                : const EmptyBestieAvatar()
          : usesInsetAgentArtwork(profile?.avatarUrl)
          ? PresetAgentAvatar(
              imageUrl: profile!.avatarUrl!,
              size: 47.88,
              fallback: Center(child: Text(profile.initial)),
            )
          : AvatarImage(
              imageUrl: profile?.avatarUrl,
              radius: 18,
              isAgent: true,
              backgroundColor: context.controlSurface,
              fallback: Center(child: Text(profile?.initial ?? 'A')),
            ),
    );
    return BuzzCallout(
      message: 'Create your first agent',
      visible: showCallout,
      delay: const Duration(seconds: 3),
      onTap: open,
      onDismiss: () => dismissFirstAgentCallout(ref),
      artwork: [
        for (final character in emptyBestieCharacters.take(6))
          AvatarImageContent(
            imageUrl: character.url,
            preferThumbnail: true,
            fit: BoxFit.contain,
            fallback: const AgentNavigationIcon(size: 24),
          ),
      ],
      child: DefaultAgentVoiceControl(
        key: ValueKey((scope, pubkey)),
        draftIdentity: draftIdentity,
        enabled:
            holdToTalk &&
            pubkey != null &&
            !opening.value &&
            !preference.isLoading &&
            !preference.hasError &&
            !savedRecipients.isLoading &&
            !savedRecipients.hasError &&
            savedRecipient == null,
        onRecorded: (recording) => open(recording: recording),
        child: SizedBox.square(
          dimension: 56,
          child: IconButton(
            tooltip: label,
            padding: EdgeInsets.zero,
            onPressed: opening.value || preference.isLoading
                ? null
                : () {
                    if (savedRecipients.hasError) {
                      ref.invalidate(
                        pendingAgentVoiceRecipientsProvider(scopeIdentity),
                      );
                    }
                    unawaited(open());
                  },
            icon: avatar,
          ),
        ),
      ),
    );
  }
}
