import 'package:buzz/shared/widgets/feedback_snackbar.dart';

import 'dart:async';

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
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/agent_navigation_icon.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/preset_agent_avatar.dart';
import '../../shared/widgets/navigation_button_surface.dart';
import '../../shared/widgets/buzz_callout.dart';
import 'first_agent_callout.dart';
import 'empty_bestie_avatar.dart';
import 'channel.dart';
import 'channel_detail_page.dart';
import 'channels_provider.dart';
import 'channel_management_provider.dart';

/// Persistent shortcut to the agent chosen in Agents.
class DefaultAgentLauncher extends HookConsumerWidget {
  const DefaultAgentLauncher({required this.onChooseAgent, super.key});
  final VoidCallback onChooseAgent;
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final preference = ref.watch(defaultAgentProvider);
    final scope = ref.watch(meSyncProvider);
    final pubkey = preference.isLoading ? null : preference.value;
    final profiles = ref.watch(userCacheProvider);
    final profile = profiles[pubkey];
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
    final label = pubkey != null
        ? 'Chat with ${profile?.label ?? 'your Bestie'}'
        : preference.hasError
        ? 'Retry Bestie'
        : 'Bestie';
    Future<void> open() async {
      final currentPreference = ref.read(defaultAgentProvider);
      if (opening.value ||
          currentPreference.isLoading ||
          ref.read(meSyncProvider) != scope ||
          currentPreference.value != pubkey) {
        return;
      }
      unawaited(HapticFeedback.lightImpact());
      if (preference.hasError) {
        ref.invalidate(defaultAgentProvider);
        return;
      }
      if (pubkey == null) {
        if (remoteAgents) dismissFirstAgentCallout(ref);
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
        final channel =
            cached ??
            await ref.read(channelActionsProvider).openDm(pubkeys: [pubkey]);
        if (context.mounted) resolving.value = false;
        if (!context.mounted) return;
        // A community switch invalidates an in-flight destination.
        if (ref.read(meSyncProvider) != scope ||
            ref.read(defaultAgentProvider).isLoading ||
            ref.read(defaultAgentProvider).value != pubkey) {
          return;
        }
        await Navigator.of(context).push<void>(
          MaterialPageRoute(
            // Native glass and video layers must track the live route.
            allowSnapshotting: false,
            builder: (_) => ChannelDetailPage(channel: channel),
          ),
        );
      } catch (error) {
        if (context.mounted) {
          ScaffoldMessenger.of(context).showFeedbackSnackBar(
            SnackBar(
              content: Text('Could not open the agent conversation. $error'),
            ),
            kind: ToastKind.error,
          );
        }
      } finally {
        if (context.mounted) {
          opening.value = false;
          resolving.value = false;
        }
      }
    }

    return BuzzCallout(
      message: 'Create your first agent',
      visible: showCallout,
      // Give people a moment with Home before inviting them.
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
      child: NavigationButtonSurface(
        child: SizedBox.square(
          dimension: 56,
          child: IconButton(
            tooltip: label,
            padding: EdgeInsets.zero,
            onPressed: opening.value || preference.isLoading ? null : open,
            icon: ExcludeSemantics(
              child: resolving.value
                  ? const SizedBox.square(
                      dimension: 22,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    )
                  : pubkey == null
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
            ),
          ),
        ),
      ),
    );
  }
}
