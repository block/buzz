import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/identity_names/identity_names_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import 'channel.dart';
import 'channel_detail_page.dart';
import 'channel_management_provider.dart';
import 'compose_bar.dart';
import 'dm_channel_labels.dart';
import 'send_message_provider.dart';

/// A new DM to [pubkeys]: no DM exists until the first send, which creates it
/// and then shows it in place of this draft.
class DmDraftPage extends HookConsumerWidget {
  const DmDraftPage({required this.pubkeys, super.key});

  /// The recipients, lowercased and sorted; never including you.
  final List<String> pubkeys;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final me = ref.watch(currentPubkeyProvider);
    final originalMe = useMemoized(() => me);
    final created = useState<Channel?>(null);
    // A DM opened by a send that then failed: retrying only sends again.
    final opened = useRef<Channel?>(null);
    final sending = useState(false);
    final error = useState<String?>(null);
    final profiles = ref.watch(userCacheProvider);
    final names = watchIdentityNames(
      ref,
      pubkeys,
      agentPubkeys: {
        for (final pubkey in pubkeys)
          if (profiles[pubkey]?.isAgent == true) pubkey,
      },
    );
    final title = formatDmParticipantDisplayName([
      for (final pubkey in pubkeys) names.labelFor(pubkey),
    ]);

    if (created.value case final channel?) {
      return ChannelDetailPage(
        channel: channel,
        placement: ChannelRoutePlacement.ordinary,
      );
    }

    Future<void> send(
      String content,
      List<String> mentions, {
      List<List<String>> mediaTags = const [],
    }) async {
      if (sending.value) throw Exception('Wait for this message to send.');
      if (me == null || me != originalMe) {
        throw Exception('Your account changed. Start this message again.');
      }
      sending.value = true;
      error.value = null;
      try {
        final channel =
            opened.value ??
            await ref
                .read(channelActionsProvider)
                .openDm(pubkeys: pubkeys, waitForChannelList: false);
        opened.value = channel;
        await ref
            .read(sendMessageProvider)
            .call(
              channelId: channel.id,
              content: content,
              mentionPubkeys: mentions,
              channel: channel,
              mediaTags: mediaTags,
            );
        if (context.mounted) created.value = channel;
      } catch (failure) {
        final message = '$failure'.replaceFirst('Bad state: ', '');
        if (context.mounted) error.value = message;
        // ComposeBar treats StateError as a community switch. Keep the draft
        // through its ordinary error path instead.
        throw Exception(message);
      } finally {
        if (context.mounted) sending.value = false;
      }
    }

    return PopScope(
      canPop: !sending.value,
      child: FrostedScaffold(
        appBar: FrostedAppBar(
          leading: BackButton(
            onPressed: () => Navigator.of(context).maybePop(),
          ),
          nativeLeading: IosNavigationAction(
            label: 'Back',
            symbol: 'chevron.backward',
            onPressed: () => Navigator.of(context).maybePop(),
          ),
          title: Text(title),
          nativeTitle: title,
          centerTitle: true,
        ),
        body: Padding(
          padding: EdgeInsets.only(top: frostedAppBarHeight(context)),
          child: Column(
            children: [
              Expanded(
                child: Center(
                  child: Padding(
                    padding: const EdgeInsets.all(Grid.gutter),
                    child: Text(
                      'Your DM with $title starts when you send.',
                      textAlign: TextAlign.center,
                      style: context.textTheme.bodyMedium?.copyWith(
                        color: context.colors.onSurfaceVariant,
                      ),
                    ),
                  ),
                ),
              ),
              if (error.value != null)
                Padding(
                  padding: const EdgeInsets.all(Grid.gutter),
                  child: Semantics(liveRegion: true, child: Text(error.value!)),
                ),
              if (me != null)
                ComposeBar(
                  channelId: 'new-dm:${pubkeys.join(',')}',
                  hintText: 'Message $title',
                  draftMembers: [
                    ChannelMember(
                      pubkey: me,
                      role: 'owner',
                      joinedAt: DateTime(2026),
                    ),
                    for (final pubkey in pubkeys)
                      ChannelMember(
                        pubkey: pubkey,
                        role: profiles[pubkey]?.isAgent == true
                            ? 'bot'
                            : 'member',
                        joinedAt: DateTime(2026),
                        displayName: names.labelFor(pubkey),
                      ),
                  ],
                  onSend: send,
                ),
            ],
          ),
        ),
      ),
    );
  }
}
