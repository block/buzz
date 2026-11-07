import 'package:flutter/widgets.dart';

import '../../../../shared/mentions/agent_identity_provider.dart';
import '../../../../shared/slash_commands/slash_errors.dart';
import '../../channel_lifecycle.dart';
import '../../channel_management_provider.dart';
import '../../channel_mutes/channel_mutes_provider.dart';
import '../slash_command.dart';

bool _canManageChannel(SlashVisibilityContext v) => canManageChannelLifecycle(
  members: v.ref.watch(channelMembersProvider(v.channelId)).value ?? const [],
  agentOwners: v.ref.watch(agentOwnersProvider).value ?? const {},
  currentPubkey: v.ref.watch(currentPubkeyProvider),
);

Future<void> _rename(SlashCommandContext ctx) async {
  final name = canonicalChannelName(ctx.args);
  if (name.isEmpty) throw const SlashUsageError();
  if (name == ctx.channel?.name.trim()) {
    throw SlashCommandFailure('This channel is already named #$name');
  }
  final actions = ctx.ref.read(channelActionsProvider);
  await actions.updateChannel(channelId: ctx.channelId, name: name);
  ctx.showMessage('Renamed to #$name');
}

Future<void> _archive(SlashCommandContext ctx) async {
  final actions = ctx.ref.read(channelActionsProvider);
  final name = ctx.channel?.name ?? 'this channel';
  final confirmed = await confirmChannelAction(
    ctx.context,
    title: 'Archive #$name?',
    body: archiveChannelConfirmBody,
    confirmLabel: 'Archive',
  );
  if (!confirmed) throw const SlashCommandCancelled();
  await actions.archiveChannel(ctx.channelId);
  if (ctx.context.mounted) {
    Navigator.of(ctx.context).popUntil((route) => route.isFirst);
  }
}

Future<void> _mute(SlashCommandContext ctx) async {
  final notifier = ctx.ref.read(channelMutesProvider.notifier);
  final muted =
      ctx.ref.read(channelMutesProvider).store.channels[ctx.channelId]?.muted ==
      true;
  final label = ctx.channel == null ? 'channel' : '#${ctx.channel!.name}';
  if (muted) {
    notifier.unmuteChannel(ctx.channelId);
    ctx.showMessage('Unmuted $label');
  } else {
    notifier.muteChannel(ctx.channelId);
    ctx.showMessage('Muted $label');
  }
}

final List<SlashCommand> channelAdminSlashCommands = [
  SlashCommand(
    name: 'rename',
    argumentHint: '<name>',
    description: 'Rename this channel',
    usage: '/rename <name>',
    scope: SlashScope.channel,
    isVisible: _canManageChannel,
    run: _rename,
  ),
  SlashCommand(
    name: 'archive',
    description: 'Archive this channel',
    usage: '/archive',
    scope: SlashScope.channel,
    isVisible: _canManageChannel,
    run: _archive,
  ),
  SlashCommand(
    name: 'mute',
    description: 'Mute or unmute this channel',
    usage: '/mute',
    run: _mute,
  ),
];
