import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'channel_management_provider.dart';
import '../../shared/widgets/adaptive_conversation_pane.dart';
import 'channel.dart';
import 'channel_detail_page.dart';
import 'channels_provider.dart';
import 'dm_draft_page.dart';

/// Where messaging some people leads: their DM, or a draft to them. Only a
/// send creates a DM, so choosing someone by mistake never leaves them an
/// empty conversation.
sealed class DmDestination {
  const DmDestination();
}

/// The DM these people already share with you.
final class ExistingDm extends DmDestination {
  const ExistingDm(this.channel);
  final Channel channel;
}

/// A new message to these people; their DM is created on its first send.
final class DmDraft extends DmDestination {
  DmDraft(Iterable<String> pubkeys)
    : pubkeys = List.unmodifiable(
        {for (final pubkey in pubkeys) pubkey.trim().toLowerCase()}.toList()
          ..sort(),
      );
  final List<String> pubkeys;
}

/// Your visible DM with exactly [pubkeys], if one is listed.
Channel? existingDmWith(
  Iterable<Channel> channels,
  String currentPubkey,
  Iterable<String> pubkeys,
) {
  final self = currentPubkey.trim().toLowerCase();
  final wanted = {
    self,
    for (final pubkey in pubkeys) pubkey.trim().toLowerCase(),
  };
  for (final channel in channels) {
    if (!channel.isDm || channel.isArchived || !channel.isMember) continue;
    final participants = {
      for (final pubkey in channel.participantPubkeys)
        if (pubkey.trim().isNotEmpty) pubkey.trim().toLowerCase(),
    };
    if (participants.length == wanted.length &&
        participants.containsAll(wanted)) {
      return channel;
    }
  }
  return null;
}

/// Your DM with [pubkeys] if the channel list has it, otherwise a draft. A
/// hidden DM is not listed, so its draft's first send reopens it.
DmDestination dmDestination(WidgetRef ref, Iterable<String> pubkeys) {
  final self = ref.read(currentPubkeyProvider);
  final channel = self == null
      ? null
      : existingDmWith(
          ref.read(channelsProvider).value ?? const [],
          self,
          pubkeys,
        );
  return channel != null ? ExistingDm(channel) : DmDraft(pubkeys);
}

/// Opens [destination] in the conversation pane, or pushes it.
Future<void> openDmDestination(
  BuildContext context,
  DmDestination destination,
) async {
  final (String id, WidgetBuilder builder) = switch (destination) {
    ExistingDm(:final channel) => (
      channel.id,
      (_) => ChannelDetailPage(
        channel: channel,
        placement: ChannelRoutePlacement.ordinary,
      ),
    ),
    DmDraft(:final pubkeys) => (
      'new-dm:${pubkeys.join(',')}',
      (_) => DmDraftPage(pubkeys: pubkeys),
    ),
  };
  await openConversationPane(context, id, builder);
}
