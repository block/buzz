import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../channels/channel_messages_provider.dart';
import 'huddle_audio.dart';
import 'huddle_controller.dart';

String? activeHuddleChannelId(Iterable<NostrEvent> events) {
  final sorted =
      events
          .where(
            (event) =>
                event.kind >= EventKind.huddleStarted &&
                event.kind <= EventKind.huddleEnded,
          )
          .toList()
        ..sort((a, b) {
          final created = a.createdAt.compareTo(b.createdAt);
          if (created != 0) return created;
          final kind = a.kind.compareTo(b.kind);
          return kind != 0 ? kind : a.id.compareTo(b.id);
        });
  final active = <String, NostrEvent>{};
  for (final event in sorted) {
    final Object? decoded;
    try {
      decoded = jsonDecode(event.content);
    } catch (_) {
      continue;
    }
    if (decoded is! Map<String, dynamic>) continue;
    final channelId = decoded['ephemeral_channel_id'];
    if (channelId is! String || channelId.isEmpty) continue;
    switch (event.kind) {
      case EventKind.huddleStarted:
        active[channelId] = event;
        break;
      case EventKind.huddleParticipantJoined:
      case EventKind.huddleParticipantLeft:
        break;
      case EventKind.huddleEnded:
        active.remove(channelId);
        break;
    }
  }
  if (active.isEmpty) return null;
  final rooms = active.entries.toList()
    ..sort((a, b) {
      final created = a.value.createdAt.compareTo(b.value.createdAt);
      return created != 0 ? created : a.value.id.compareTo(b.value.id);
    });
  return rooms.last.key;
}

class HuddleButton extends ConsumerWidget {
  const HuddleButton({required this.channelId, super.key});
  final String channelId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final state = ref.watch(huddleControllerProvider);
    final messages =
        ref.watch(channelMessagesProvider(channelId)).value ?? const [];
    final joinableChannelId = activeHuddleChannelId(messages);
    final activeHere = state.parentChannelId == channelId && state.isActive;
    final canJoin = !state.isActive && joinableChannelId != null;
    return IconButton(
      tooltip: activeHere
          ? 'Huddle controls'
          : canJoin
          ? 'Join Huddle'
          : 'Start Huddle',
      color: activeHere || canJoin
          ? context.appColors.success
          : context.colors.primary,
      icon: Icon(
        activeHere || canJoin ? LucideIcons.audioLines : LucideIcons.phone,
        size: 22,
      ),
      onPressed: () {
        if (!state.isActive) {
          final controller = ref.read(huddleControllerProvider.notifier);
          if (joinableChannelId != null) {
            controller
                .join(parentChannelId: channelId, channelId: joinableChannelId)
                .catchError((Object _) {});
          } else {
            controller.start(channelId).catchError((Object _) {});
          }
        }
        showModalBottomSheet<void>(
          context: context,
          showDragHandle: true,
          builder: (_) => const HuddleControlsSheet(),
        );
      },
    );
  }
}

class HuddleControlsSheet extends ConsumerWidget {
  const HuddleControlsSheet({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final state = ref.watch(huddleControllerProvider);
    final controller = ref.read(huddleControllerProvider.notifier);
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.all(Grid.lg),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text('Huddle', style: context.textTheme.titleLarge),
            const SizedBox(height: Grid.sm),
            Text(_status(state), textAlign: TextAlign.center),
            if (state.error != null) ...[
              const SizedBox(height: Grid.sm),
              Text(
                state.error!,
                style: TextStyle(color: context.colors.error),
                textAlign: TextAlign.center,
              ),
            ],
            if (state.peers.isNotEmpty) ...[
              const SizedBox(height: Grid.sm),
              Text(
                '${state.peers.length} participant${state.peers.length == 1 ? '' : 's'}',
              ),
            ],
            if (state.botPubkeys.isNotEmpty) ...[
              const SizedBox(height: Grid.sm),
              Text(
                state.isTranscribing
                    ? 'Listening for your bot question…'
                    : state.isBotSpeaking
                    ? 'Bot is speaking…'
                    : '${state.botPubkeys.length} bot${state.botPubkeys.length == 1 ? '' : 's'} ready',
                textAlign: TextAlign.center,
              ),
              if (state.lastTranscript != null)
                Text(
                  'You: ${state.lastTranscript}',
                  maxLines: 2,
                  overflow: TextOverflow.ellipsis,
                  textAlign: TextAlign.center,
                  style: context.textTheme.bodySmall,
                ),
            ],
            const SizedBox(height: Grid.md),
            Row(
              mainAxisAlignment: MainAxisAlignment.spaceEvenly,
              children: [
                IconButton.filledTonal(
                  tooltip: state.isMuted ? 'Unmute' : 'Mute',
                  onPressed: state.isActive ? controller.toggleMute : null,
                  icon: Icon(
                    state.isMuted ? LucideIcons.micOff : LucideIcons.mic,
                  ),
                ),
                PopupMenuButton<HuddleOutputRoute>(
                  tooltip: 'Audio output',
                  initialValue: state.outputRoute,
                  enabled: state.isActive,
                  onSelected: (route) => controller.setOutputRoute(route),
                  itemBuilder: (_) => HuddleOutputRoute.values
                      .map(
                        (route) => PopupMenuItem(
                          value: route,
                          child: Text(route.name),
                        ),
                      )
                      .toList(),
                  icon: const Icon(LucideIcons.volume2),
                ),
                IconButton.filledTonal(
                  tooltip: state.isTranscribing
                      ? 'Finish bot voice turn'
                      : 'Talk to bots',
                  onPressed:
                      state.phase == HuddlePhase.connected &&
                          state.botPubkeys.isNotEmpty
                      ? () => controller.talkToBots().catchError((Object _) {})
                      : null,
                  icon: Icon(
                    state.isTranscribing
                        ? LucideIcons.square
                        : LucideIcons.messageCircleMore,
                  ),
                ),
                IconButton.filled(
                  tooltip: 'Leave Huddle',
                  style: IconButton.styleFrom(
                    backgroundColor: context.colors.error,
                  ),
                  onPressed: state.isActive
                      ? () async {
                          await controller.leave();
                          if (context.mounted) Navigator.of(context).pop();
                        }
                      : null,
                  icon: const Icon(LucideIcons.phoneOff),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }

  String _status(HuddleState state) => switch (state.phase) {
    HuddlePhase.idle => 'Not connected',
    HuddlePhase.creating => 'Starting…',
    HuddlePhase.connecting => 'Connecting…',
    HuddlePhase.connected => state.isMuted ? 'Connected · muted' : 'Connected',
    HuddlePhase.reconnecting => 'Network lost · reconnecting…',
    HuddlePhase.failed => 'Could not connect',
  };
}
