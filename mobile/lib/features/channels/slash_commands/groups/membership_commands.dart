import 'dart:async';

import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../../shared/slash_commands/slash_errors.dart';
import '../../../../shared/widgets/modal_presentation.dart';
import '../../../profile/user_profile_sheet.dart';
import '../../channel_detail_page.dart';
import '../../channel_identity_names_provider.dart';
import '../../channel_management_provider.dart';
import '../../channel_member_profile_actions.dart';
import '../../channels_provider.dart';
import '../../members_sheet.dart';
import '../slash_channel_refs.dart';
import '../slash_command.dart';

final List<SlashCommand> membershipSlashCommands = [
  SlashCommand(
    name: 'invite',
    argumentHint: '@user…',
    description: 'Add people to this channel',
    usage: '/invite @user…',
    scope: SlashScope.channel,
    isVisible: (v) {
      final channel = v.channel;
      if (channel == null) return true;
      if (channel.isArchived) return false;
      final roster = v.ref.watch(channelMembersProvider(v.channelId));
      final members = roster.asData?.value;
      if (members == null) return true;
      return channel.canAddMembers(
        _roleOf(members, v.ref.watch(currentPubkeyProvider)),
      );
    },
    run: _invite,
  ),
  SlashCommand(
    name: 'remove',
    aliases: ['kick'],
    argumentHint: '@user…',
    description: 'Remove people from this channel',
    usage: '/remove @user…',
    scope: SlashScope.channel,
    isVisible: (v) {
      if (v.channel?.isArchived ?? false) return false;
      final roster = v.ref.watch(channelMembersProvider(v.channelId));
      final members = roster.asData?.value;
      if (members == null) return true;
      final role = _roleOf(members, v.ref.watch(currentPubkeyProvider));
      return role == 'owner' || role == 'admin';
    },
    run: _remove,
  ),
  SlashCommand(
    name: 'join',
    argumentHint: '#channel',
    description: 'Join a channel and open it',
    usage: '/join #channel',
    run: _join,
  ),
  SlashCommand(
    name: 'leave',
    aliases: ['part'],
    description: 'Leave this channel',
    usage: '/leave',
    scope: SlashScope.channel,
    run: _leave,
  ),
  SlashCommand(
    name: 'who',
    aliases: ['members'],
    description: 'Show who is in this channel',
    usage: '/who',
    run: _who,
  ),
];

String? _roleOf(List<ChannelMember> roster, String? pubkey) {
  if (pubkey == null) return null;
  final self = pubkey.toLowerCase();
  for (final member in roster) {
    if (member.pubkey.toLowerCase() == self) return member.role;
  }
  return null;
}

Set<String> _mentioned(SlashCommandContext ctx) {
  if (ctx.mentions.isEmpty) throw const SlashUsageError();
  return {for (final m in ctx.mentions) m.pubkey.toLowerCase()};
}

Future<void> _invite(SlashCommandContext ctx) async {
  final mentioned = _mentioned(ctx);
  final actions = ctx.ref.read(channelActionsProvider);
  final roster = ctx.ref.read(channelMembersProvider(ctx.channelId).future);

  final members = {for (final m in await roster) m.pubkey.toLowerCase()};
  final pubkeys = mentioned.difference(members).toList();
  if (pubkeys.isEmpty) {
    throw const SlashCommandFailure('Already in this channel');
  }
  try {
    await actions.addMembers(channelId: ctx.channelId, pubkeys: pubkeys);
  } on AddMembersException catch (error) {
    throw SlashCommandFailure(error.message);
  }
  ctx.showMessage('Added ${pubkeys.length}');
}

Future<void> _remove(SlashCommandContext ctx) async {
  final mentioned = _mentioned(ctx);
  final self = ctx.ref.read(currentPubkeyProvider)?.toLowerCase();
  if (self != null && mentioned.contains(self)) {
    throw const SlashCommandFailure('Use /leave to leave this channel');
  }
  final actions = ctx.ref.read(channelActionsProvider);
  final roster = ctx.ref.read(channelMembersProvider(ctx.channelId).future);

  final members = {for (final m in await roster) m.pubkey.toLowerCase()};
  final pubkeys = mentioned.intersection(members).toList();
  if (pubkeys.isEmpty) {
    throw const SlashCommandFailure('Not in this channel');
  }
  for (final pubkey in pubkeys) {
    await actions.removeMember(channelId: ctx.channelId, pubkey: pubkey);
  }
  ctx.showMessage('Removed ${pubkeys.length}');
}

Future<void> _join(SlashCommandContext ctx) async {
  final navigator = Navigator.of(ctx.context);
  final channels = ctx.ref.read(channelsProvider.notifier);
  final actions = ctx.ref.read(channelActionsProvider);

  var refs = (channels: ctx.channels, unresolved: ctx.unresolvedChannelNames);
  if (refs.unresolved.isNotEmpty) {
    await channels.ensureDirectoryLoaded();
    if (!ctx.context.mounted) return;
    if (ctx.ref.read(channelDirectoryLoadStatusProvider).status ==
        ChannelDirectoryLoadStatus.error) {
      throw const SlashCommandFailure('Couldn’t load the channel directory');
    }
    refs = resolveSlashChannelRefs(
      ctx.args,
      ctx.ref.read(channelsProvider).asData?.value ?? const [],
    );
  }
  if (refs.unresolved.isNotEmpty) {
    throw SlashUsageError('No channel named #${refs.unresolved.first}');
  }
  if (refs.channels.isEmpty) throw const SlashUsageError();
  final target = refs.channels.first;

  var opened = target;
  if (!target.isMember) {
    if (!target.canJoin) {
      throw SlashCommandFailure(
        target.isArchived
            ? '#${target.name} is archived'
            : '#${target.name} is private',
      );
    }
    await actions.joinChannel(target.id);
    final refreshed = channels.channelById(target.id);
    opened = refreshed != null && refreshed.isMember
        ? refreshed
        : target.copyWith(isMember: true);
  } else if (target.id == ctx.channelId) {
    return;
  }
  if (!navigator.mounted) return;
  unawaited(
    navigator.push(
      MaterialPageRoute<void>(
        builder: (_) => ChannelDetailPage(
          channel: opened,
          placement: ChannelRoutePlacement.ordinary,
        ),
      ),
    ),
  );
}

Future<void> _leave(SlashCommandContext ctx) async {
  final channel =
      ctx.channel ??
      ctx.ref.read(channelsProvider.notifier).channelById(ctx.channelId);
  if (channel == null) throw const SlashCommandFailure('Channel not found');
  final navigator = Navigator.of(ctx.context);
  final actions = ctx.ref.read(channelActionsProvider);

  final confirmed = await showBuzzDialog<bool>(
    context: ctx.context,
    builder: (dialogContext) => AlertDialog(
      title: Text('Leave #${channel.name}?'),
      content: const Text('You’ll stop receiving messages from this channel.'),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(dialogContext).pop(false),
          child: const Text('Cancel'),
        ),
        FilledButton(
          onPressed: () => Navigator.of(dialogContext).pop(true),
          child: const Text('Leave'),
        ),
      ],
    ),
  );
  if (confirmed != true) return;
  await actions.leaveChannel(channel.id);
  if (!navigator.mounted) return;
  if (ctx.inThread) {
    navigator.popUntil((route) => route.isFirst);
  } else {
    navigator.pop();
  }
}

Future<void> _who(SlashCommandContext ctx) async {
  final channel =
      ctx.channel ??
      ctx.ref.read(channelsProvider.notifier).channelById(ctx.channelId);
  if (channel == null) throw const SlashCommandFailure('Channel not found');
  final currentPubkey = ctx.ref.read(currentPubkeyProvider);
  unawaited(
    showBuzzModalBottomSheet<void>(
      context: ctx.context,
      title: 'Members',
      isScrollControlled: true,
      showDragHandle: true,
      builder: (_) => MembersSheet(
        channel: channel,
        currentPubkey: currentPubkey,
        onMemberTap: (context, pubkey) => showUserProfileSheet(
          context,
          pubkey,
          names: channelIdentityNamesProvider(channel.id),
          contextualActions: (_) =>
              ChannelMemberProfileActions(channel: channel, pubkey: pubkey),
        ),
      ),
    ),
  );
}
