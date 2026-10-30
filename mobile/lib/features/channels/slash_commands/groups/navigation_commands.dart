import 'package:flutter/material.dart';

import '../../../../shared/custom_emoji/custom_emoji.dart';
import '../../../../shared/custom_emoji/custom_emoji_provider.dart';
import '../../../../shared/mentions/mention_bindings.dart';
import '../../../../shared/relay/relay_provider.dart';
import '../../../../shared/huddle/huddle_session.dart';
import '../../../../shared/slash_commands/slash_errors.dart';
import '../../channel_detail_page.dart';
import '../../channel_management_provider.dart';
import '../../send_message_provider.dart';
import '../slash_command.dart';

/// The relay rejects a DM with more than 8 other participants.
const maxDmOtherParticipants = 8;

/// The mentions that open with [args], and the text after the last of them.
/// A mention counts only when nothing but whitespace precedes it.
({List<SlashMention> recipients, String message}) parseDmArgs(
  String args,
  List<SlashMention> mentions,
) {
  final byLabel = {
    for (final mention in mentions) mention.label.toLowerCase(): mention,
  };
  final recipients = <SlashMention>[];
  var end = 0;
  for (final range in mentionOccurrences(args, byLabel.keys)) {
    if (args.substring(end, range.start).trim().isNotEmpty) break;
    recipients.add(byLabel[range.label]!);
    end = range.end;
  }
  return (recipients: recipients, message: args.substring(end).trim());
}

Future<void> _dm(SlashCommandContext ctx) async {
  final parsed = parseDmArgs(ctx.args, ctx.mentions);
  final self = ctx.ref.read(myPubkeyProvider)?.toLowerCase();
  final pubkeys = <String>{
    for (final mention in parsed.recipients)
      if (mention.pubkey.toLowerCase() != self) mention.pubkey.toLowerCase(),
  }.toList();
  if (pubkeys.isEmpty) throw const SlashUsageError('Mention who to message');
  if (pubkeys.length > maxDmOtherParticipants) {
    throw const SlashCommandFailure(
      'A DM can include at most $maxDmOtherParticipants other people',
    );
  }

  final actions = ctx.ref.read(channelActionsProvider);
  final send = ctx.ref.read(sendMessageProvider);
  final customEmoji = ctx.ref.read(customEmojiListProvider);
  final navigator = Navigator.of(ctx.context);

  final dm = await actions.openDm(pubkeys: pubkeys, waitForChannelList: false);
  if (parsed.message.isNotEmpty) {
    await send(
      channelId: dm.id,
      content: parsed.message,
      channel: dm,
      mentionPubkeys: const [],
      mediaTags: buildCustomEmojiTags(parsed.message, customEmoji),
    );
  }
  if (!ctx.context.mounted || (!ctx.inThread && dm.id == ctx.channelId)) {
    return;
  }
  await navigator.push(
    MaterialPageRoute<void>(
      builder: (_) => ChannelDetailPage(
        channel: dm,
        placement: ChannelRoutePlacement.ordinary,
      ),
    ),
  );
}

Future<void> _huddle(SlashCommandContext ctx) async {
  final session = ctx.ref.read(huddleSessionProvider);
  if (session.isInSession && session.parentChannelId != ctx.channelId) {
    throw const SlashCommandFailure('Leave your current Huddle first');
  }
  if (ctx.channel?.isMember == false || ctx.channel?.isArchived == true) {
    throw const SlashCommandFailure(
      'Huddles need an active channel you belong to',
    );
  }
  try {
    await startOrOpenChannelHuddle(
      context: ctx.context,
      ref: ctx.ref,
      channelId: ctx.channelId,
    );
  } on StateError catch (error) {
    throw SlashCommandFailure(error.message);
  }
}

final List<SlashCommand> navigationSlashCommands = [
  SlashCommand(
    name: 'dm',
    aliases: const ['msg'],
    argumentHint: '@user… [message]',
    description: 'Open a direct message, optionally sending a message first',
    usage: '/dm @user… [message]',
    run: _dm,
  ),
  SlashCommand(
    name: 'huddle',
    description: 'Start a huddle in this channel, or join the one running',
    usage: '/huddle',
    run: _huddle,
  ),
];
