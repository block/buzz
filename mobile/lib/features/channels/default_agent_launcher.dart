import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/me/default_agent.dart';
import '../../shared/animated_avatar.dart';
import '../../shared/me/me_sync.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/agent_navigation_icon.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/preset_agent_avatar.dart';
import 'channel.dart';
import 'default_agent_voice_control.dart';
import 'default_agent_voice_draft.dart';
import 'voice_note_recording.dart';
import 'send_message_provider.dart';
import 'channel_detail_page.dart';
import 'channels_provider.dart';
import 'channel_management_provider.dart';

/// Persistent chat and voice shortcut to the agent chosen in Agents.
class DefaultAgentLauncher extends HookConsumerWidget {
  const DefaultAgentLauncher({required this.onChooseAgent, super.key});
  final VoidCallback onChooseAgent;
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final preference = ref.watch(defaultAgentProvider);
    final scope = ref.watch(meSyncProvider);
    final scopeIdentity =
        '${scope.community}:${ref.watch(myPubkeyProvider) ?? 'signed-out'}';
    final savedRecipients = ref.watch(
      pendingAgentVoiceRecipientsProvider(scopeIdentity),
    );
    final savedRecipient = savedRecipients.value?.firstOrNull;
    final pubkey =
        savedRecipient ?? (preference.isLoading ? null : preference.value);
    final profiles = ref.watch(userCacheProvider);
    final profile = profiles[pubkey];
    final opening = useState(false);
    final resolving = useState(false);
    useEffect(() {
      if (pubkey != null) {
        ref.read(userCacheProvider.notifier).preload([pubkey]);
      }
      return null;
    }, [pubkey]);
    final draftIdentity = '$scopeIdentity:$pubkey';
    final label = pubkey != null
        ? 'Chat with ${profile?.label ?? 'your Bestie'}'
        : preference.hasError
        ? 'Retry Bestie'
        : 'Bestie';
    Future<void> open({VoiceNoteRecording? recording}) async {
      bool destinationIsCurrent() {
        if (ref.read(meSyncProvider) != scope) return false;
        // An explicit retry keeps the recipient captured with the draft.
        if (recording != null) return true;
        final current = ref.read(defaultAgentProvider);
        return !current.isLoading && current.value == pubkey;
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
      if (pubkey == null) {
        if (recording != null) throw StateError('No default agent selected.');
        onChooseAgent();
        return;
      }
      opening.value = true;
      try {
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
                pubkey,
              ),
            )
            .firstOrNull;
        resolving.value = cached == null;
        final draft = ref.read(defaultAgentVoiceDraftProvider(draftIdentity));
        final uploadService = ref.read(mediaUploadServiceProvider);
        // Resolve the destination while packaging/uploading, with both errors
        // observed even if one operation fails before the other completes.
        final (channel, prepared) = await (
          cached != null
              ? Future.value(cached)
              : ref.read(channelActionsProvider).openDm(pubkeys: [pubkey]),
          () async {
            if (recording == null) return (null, null);
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
          if (recording != null) throw StateError('The agent changed.');
          return;
        }
        if (recording != null) {
          final session = ref.read(relaySessionProvider.notifier);
          final previousEvent = prepared.$1;
          if (previousEvent != null) {
            if (!context.mounted || !destinationIsCurrent()) {
              throw StateError('The community changed.');
            }
            await session.publish(previousEvent);
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
              mentionPubkeys: [pubkey],
              mediaTags: [descriptor.toImetaTag()],
              beforePublish: (event) async {
                if (!context.mounted || !destinationIsCurrent()) {
                  throw StateError('The agent or community changed.');
                }
                await draft.saveEvent(event);
              },
            );
          }
          if (!context.mounted) return;
        }
        if (!context.mounted || !destinationIsCurrent()) {
          return;
        }
        unawaited(
          Navigator.of(context).push<void>(
            MaterialPageRoute(
              // Native glass and video layers must track the live route.
              allowSnapshotting: false,
              builder: (_) => ChannelDetailPage(channel: channel),
            ),
          ),
        );
      } catch (error) {
        if (recording != null) rethrow;
        if (context.mounted) {
          ScaffoldMessenger.of(context).showSnackBar(
            SnackBar(
              content: Text('Could not open the agent conversation. $error'),
            ),
          );
        }
      } finally {
        if (context.mounted) {
          opening.value = false;
          resolving.value = false;
        }
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
          : pubkey == null
          ? const AgentNavigationIcon(size: 24)
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
    if (savedRecipients.hasError) {
      return IconButton(
        tooltip: 'Retry saved voice notes',
        onPressed: () =>
            ref.invalidate(pendingAgentVoiceRecipientsProvider(scopeIdentity)),
        icon: const Icon(Icons.refresh),
      );
    }
    return DefaultAgentVoiceControl(
      key: ValueKey((scope, pubkey)),
      draftIdentity: draftIdentity,
      enabled:
          pubkey != null &&
          !opening.value &&
          !preference.isLoading &&
          !savedRecipients.isLoading &&
          savedRecipient == null,
      onRecorded: (recording) => open(recording: recording),
      child: SizedBox.square(
        dimension: 56,
        child: IconButton(
          tooltip: label,
          padding: EdgeInsets.zero,
          onPressed: opening.value || preference.isLoading ? null : open,
          icon: avatar,
        ),
      ),
    );
  }
}
