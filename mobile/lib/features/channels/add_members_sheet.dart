import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list_card_item.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/confirmation_dialog.dart';
import '../../shared/identity_names/identity_names_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/relay/relay.dart';
import 'channel.dart';
import 'channel_management_provider.dart';
import 'ios_add_members_sheet.dart';
import 'mentions/mention_candidates.dart';
import 'mentions/mention_candidates_provider.dart';
import 'mentions/mention_ranking.dart';
import 'mentions/stable_mention_rows.dart';
import '../../shared/widgets/modal_presentation.dart';

bool _nativePickerOpen = false;

/// Opens the platform member picker, retaining the Android sheet presentation.
///
/// For a DM, [isDm] asks whether added people see its history, and
/// [onAddedToDm] receives the DM they ended up in: this one, grown, or
/// another.
Future<bool?> showAddChannelMembersSheet({
  required BuildContext context,
  required String channelId,
  required Set<String> existingPubkeys,
  bool isDm = false,
  void Function(Channel dm)? onAddedToDm,
}) async {
  if (!kIsWeb && defaultTargetPlatform == TargetPlatform.iOS) {
    if (_nativePickerOpen) return null;
    _nativePickerOpen = true;
    try {
      return await showGeneralDialog<bool>(
        context: context,
        barrierColor: Colors.transparent,
        transitionDuration: Duration.zero,
        pageBuilder: (_, _, _) => AddChannelMembersSheet(
          channelId: channelId,
          existingPubkeys: existingPubkeys,
          isDm: isDm,
          onAddedToDm: onAddedToDm,
          nativePresentation: true,
        ),
      );
    } finally {
      _nativePickerOpen = false;
    }
  }
  final mediaQuery = MediaQuery.of(context);
  return showBuzzModalBottomSheet<bool>(
    context: context,
    title: 'Add members',
    isScrollControlled: true,
    showDragHandle: true,
    constraints: BoxConstraints(
      maxWidth: 640,
      maxHeight: mediaQuery.size.height - mediaQuery.viewPadding.top - Grid.xs,
    ),
    builder: (_) => AddChannelMembersSheet(
      channelId: channelId,
      existingPubkeys: existingPubkeys,
      isDm: isDm,
      onAddedToDm: onAddedToDm,
    ),
  );
}

/// Searchable multi-select used to add people or agents to a channel.
class AddChannelMembersSheet extends HookConsumerWidget {
  const AddChannelMembersSheet({
    super.key,
    required this.channelId,
    required this.existingPubkeys,
    this.isDm = false,
    this.onAddedToDm,
    this.nativePresentation = false,
  });

  final String channelId;
  final Set<String> existingPubkeys;
  final bool isDm;
  final void Function(Channel dm)? onAddedToDm;
  final bool nativePresentation;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final queryController = useTextEditingController();
    final query = useState('');
    // A new opening after each pick, as the mention chooser starts one per
    // `@`: shown rows keep their places only within one query and opening.
    final opening = useState(0);
    final selectedUsers = useState<List<MentionCandidate>>([]);
    final isSubmitting = useState(false);
    final submitError = useState<String?>(null);
    // Not a failure: what to do next, such as after an accepted add.
    final submitNotice = useState<String?>(null);
    // The DM an accepted add landed in but that is not confirmed yet. Add
    // then opens it again instead of sending the add a second time.
    final acceptedDm = useState<String?>(null);
    // The iOS picker marks itself busy before asking Dart to add; a bump
    // rebuilds the sheet so it receives the unchanged idle state again.
    final resync = useState(0);
    final actions = useMemoized(() => ref.read(channelActionsProvider));

    // The same candidates, ranking, debounce and search cache as mention
    // autocomplete and New DM. A channel can hold agents that do not answer
    // the viewer, so those are offered too.
    final search = query.value.trim();
    final ranked = ref.watch(
      recipientCandidatesProvider((
        query: search,
        historyKey: addMembersMentionHistoryKey,
        unsharedAgents: true,
        channelId: channelId,
      )),
    );
    final searching = ref.watch(recipientSearchPendingProvider(search));
    final searchFailed = ref.watch(recipientSearchFailedProvider(search));
    final normalizedExisting = existingPubkeys
        .map((pubkey) => pubkey.toLowerCase())
        .toSet();
    final selectedPubkeys = selectedUsers.value
        .map((user) => user.pubkey.toLowerCase())
        .toSet();
    final successfulPubkeys = useState<Set<String>>(<String>{});
    final excludedPubkeys = {
      ...normalizedExisting,
      ...successfulPubkeys.value,
      ...selectedPubkeys,
    };
    final shown = useStableMentionRows(
      [
        for (final candidate in ranked)
          if (!excludedPubkeys.contains(candidate.pubkey.toLowerCase()))
            candidate,
      ],
      query: search,
      opening: opening.value,
      pending: searching,
      // Agents rank ahead of the known people an empty query lists, even
      // when they load after them. Typed rows keep their places.
      restartForNewAgents: search.isEmpty,
    );
    final availableUsers = shown.rows;
    final userCache = ref.watch(userCacheProvider);
    final currentPubkey = ref.watch(currentPubkeyProvider);
    String? detail(MentionCandidate user) =>
        recipientRowDetail(user, currentPubkey, userCache);
    String? avatarUrl(MentionCandidate user) =>
        user.avatarUrl ?? userCache[user.pubkey]?.avatarUrl;
    // Compare every shown choice with the channel's members, so a candidate
    // who shares a member's name is told apart before being added. Members
    // are only compared: their names come from the roster the details page
    // already loaded, so only the shown choices' profiles are fetched.
    final choices = [...availableUsers, ...selectedUsers.value];
    final roster =
        ref.watch(channelMembersProvider(channelId)).asData?.value ??
        const <ChannelMember>[];
    final names = watchIdentityNames(
      ref,
      [...normalizedExisting, for (final user in choices) user.pubkey],
      agentPubkeys: {
        for (final member in roster)
          if (member.isBot) member.pubkey,
        for (final user in choices)
          if (user.isAgent) user.pubkey,
      },
      fallbackNames: {
        for (final member in roster) member.pubkey: ?member.displayName,
        for (final user in choices) user.pubkey: user.pickerLabel,
      },
      shown: [for (final user in choices) user.pubkey],
    );
    String labelFor(MentionCandidate user) => names.labelFor(user.pubkey);
    void retry() => ref.invalidate(mentionUserSearchProvider(search));

    void toggleUser(MentionCandidate user) {
      if (isSubmitting.value || acceptedDm.value != null) return;
      final normalized = user.pubkey.toLowerCase();
      // The current selection, not this build's: a second tap can land
      // before the rebuild that reflects the first.
      if (selectedUsers.value.any(
        (selected) => selected.pubkey.toLowerCase() == normalized,
      )) {
        selectedUsers.value = [
          for (final selected in selectedUsers.value)
            if (selected.pubkey.toLowerCase() != normalized) selected,
        ];
      } else {
        if (shown.unavailable.contains(user.pubkey)) return;
        selectedUsers.value = [...selectedUsers.value, user];
        ref
            .read(mentionHistoryProvider.notifier)
            .remember(addMembersMentionHistoryKey, user.pubkey);
      }
      opening.value++;
      submitError.value = null;
      submitNotice.value = null;
    }

    // The relay decides who is an agent: an add first asks it, and only a
    // person joining a DM of two or more people needs the history choice.
    Future<bool?> askIncludeHistory(List<MentionCandidate> users) async {
      try {
        return await showDmHistoryChoice(
          context: context,
          title: users.length == 1
              ? 'Add ${labelFor(users.single)}?'
              : 'Add ${users.length} members?',
        );
      } finally {
        if (context.mounted) resync.value++;
      }
    }

    Future<void> addSelectedMembers() async {
      if (selectedUsers.value.isEmpty || isSubmitting.value) return;
      final submittedUsers = List<MentionCandidate>.of(selectedUsers.value);
      submitError.value = null;
      submitNotice.value = null;
      try {
        final accepted = acceptedDm.value;
        if (accepted != null) {
          isSubmitting.value = true;
          final dm = await actions.openAddedDm(accepted);
          if (!context.mounted) return;
          Navigator.of(context).pop(true);
          onAddedToDm?.call(dm);
          return;
        }
        isSubmitting.value = true;
        if (isDm) {
          final pubkeys = submittedUsers.map((user) => user.pubkey).toList();
          Channel dm;
          try {
            dm = await actions.addToDm(
              channelId: channelId,
              pubkeys: pubkeys,
              history: DmHistory.ask,
            );
          } on DmNeedsHistoryChoice {
            if (!context.mounted) return;
            isSubmitting.value = false;
            final includeHistory = await askIncludeHistory(submittedUsers);
            if (includeHistory == null || !context.mounted) return;
            isSubmitting.value = true;
            dm = await actions.addToDm(
              channelId: channelId,
              pubkeys: pubkeys,
              history: includeHistory ? DmHistory.include : DmHistory.startNew,
            );
          }
          if (!context.mounted) return;
          Navigator.of(context).pop(true);
          onAddedToDm?.call(dm);
          return;
        }
        await actions.addMembers(
          channelId: channelId,
          pubkeys: submittedUsers.map((user) => user.pubkey).toList(),
        );
        if (context.mounted) Navigator.of(context).pop(true);
      } on DmAddUnconfirmed catch (error) {
        if (!context.mounted) return;
        acceptedDm.value = error.channelId;
        submitNotice.value = error.toString();
      } on AddMembersException catch (error) {
        if (!context.mounted) return;
        final failedPubkeys = error.failures.keys
            .map((pubkey) => pubkey.toLowerCase())
            .toSet();
        successfulPubkeys.value = {
          ...successfulPubkeys.value,
          for (final user in submittedUsers)
            if (!failedPubkeys.contains(user.pubkey.toLowerCase()))
              user.pubkey.toLowerCase(),
        };
        selectedUsers.value = [
          for (final user in submittedUsers)
            if (failedPubkeys.contains(user.pubkey.toLowerCase())) user,
        ];
        submitError.value = error.message;
      } catch (error) {
        if (!context.mounted) return;
        submitError.value = error.toString().replaceFirst('Exception: ', '');
      } finally {
        if (context.mounted) isSubmitting.value = false;
      }
    }

    if (nativePresentation) {
      final auth = ref.watch(mediaGetAuthServiceProvider);
      final client = ref.watch(mediaHttpClientProvider);
      final diskCache = ref.watch(mediaDiskCacheProvider);
      final colors = Theme.of(context).colorScheme;
      String avatarKey(MentionCandidate user) => jsonEncode([
        user.pubkey,
        avatarUrl(user),
        user.initial,
        user.isAgent,
        colors.primaryContainer.toARGB32(),
        colors.onPrimaryContainer.toARGB32(),
      ]);
      Map<String, Object?> row(MentionCandidate user) => {
        'pubkey': user.pubkey,
        'name': labelFor(user),
        'detail': detail(user),
        'agent': user.isAgent,
        'unavailable': shown.unavailable.contains(user.pubkey),
        'avatarKey': avatarKey(user),
        'initial': user.initial,
        'avatarBackground': colors.primaryContainer.toARGB32(),
        'avatarForeground': colors.onPrimaryContainer.toARGB32(),
        'selected': selectedPubkeys.contains(user.pubkey.toLowerCase()),
      };
      return IosAddMembersSheet(
        onAvatar: (pubkey, key) async {
          final user = choices
              .where((user) => user.pubkey == pubkey)
              .firstOrNull;
          if (user == null || avatarKey(user) != key) return null;
          return nativeAvatarImage(
            url: avatarUrl(user),
            initial: user.initial,
            isAgent: user.isAgent,
            background: colors.primaryContainer,
            foreground: colors.onPrimaryContainer,
            networkImages: (url) => smallMediaImageProviders(
              url: url,
              auth: auth,
              client: client,
              diskCache: diskCache,
            ),
          );
        },
        state: {
          'query': query.value,
          'emptyQueryHint': 'Type a name to find people or agents.',
          'loading': searching,
          'loadError': searchFailed,
          'error': submitError.value,
          'notice': submitNotice.value,
          'submitting': isSubmitting.value,
          'selected': [for (final user in selectedUsers.value) row(user)],
          'users': [for (final user in availableUsers) row(user)],
        },
        onQuery: (value) {
          if (!isSubmitting.value) query.value = value;
        },
        onToggle: (pubkey) {
          final user = choices
              .where((user) => user.pubkey == pubkey)
              .firstOrNull;
          if (user != null) toggleUser(user);
        },
        onSubmit: () => unawaited(addSelectedMembers()),
        onRetry: retry,
        onClose: () => Navigator.of(context).pop(false),
      );
    }

    return Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        0,
        Grid.gutter,
        MediaQuery.viewInsetsOf(context).bottom,
      ),
      child: SafeArea(
        top: false,
        child: ConstrainedBox(
          constraints: BoxConstraints(
            maxHeight: MediaQuery.sizeOf(context).height * 0.72,
          ),
          child: Column(
            children: [
              TextField(
                key: const ValueKey('add-channel-members-search'),
                controller: queryController,
                autofocus: true,
                autocorrect: false,
                enableSuggestions: false,
                enabled: !isSubmitting.value,
                onChanged: (value) => query.value = value,
                decoration: const InputDecoration(
                  hintText: 'Search for people or agents',
                  prefixIcon: Icon(BuzzIcons.search),
                ),
              ),
              if (selectedUsers.value.isNotEmpty) ...[
                const SizedBox(height: Grid.xxs),
                Align(
                  alignment: Alignment.centerLeft,
                  child: Wrap(
                    spacing: Grid.half,
                    runSpacing: Grid.half,
                    children: [
                      for (final user in selectedUsers.value)
                        InputChip(
                          key: ValueKey(
                            'add-channel-member-selected-${user.pubkey}',
                          ),
                          label: Text(labelFor(user)),
                          onDeleted: isSubmitting.value
                              ? null
                              : () => toggleUser(user),
                        ),
                    ],
                  ),
                ),
              ],
              const SizedBox(height: Grid.xxs),
              Expanded(
                child: availableUsers.isEmpty && !searchFailed
                    ? Center(
                        child: searching
                            ? const BuzzLoadingIndicator(
                                size: 44,
                                semanticLabel: 'Searching people and agents',
                              )
                            : Text(
                                search.isEmpty
                                    ? 'Type a name to find people or agents.'
                                    : 'No matching people or agents.',
                                textAlign: TextAlign.center,
                              ),
                      )
                    : ListView.builder(
                        key: const ValueKey('add-channel-members-results'),
                        itemCount:
                            availableUsers.length + (searchFailed ? 1 : 0),
                        itemBuilder: (context, index) {
                          if (index == availableUsers.length) {
                            return ListTile(
                              key: const ValueKey(
                                'add-channel-members-search-error',
                              ),
                              title: Text(
                                'Couldn’t search people or agents.',
                                style: context.textTheme.bodyMedium?.copyWith(
                                  color: context.colors.error,
                                ),
                              ),
                              trailing: TextButton(
                                onPressed: retry,
                                child: const Text('Retry'),
                              ),
                            );
                          }
                          final user = availableUsers[index];
                          final available = !shown.unavailable.contains(
                            user.pubkey,
                          );
                          final info = detail(user);
                          return AppListCardItem(
                            index: index,
                            itemCount: availableUsers.length,
                            dividerIndent: Grid.xs + 40 + Grid.xs,
                            child: Semantics(
                              key: ValueKey(
                                'add-channel-member-${user.pubkey}',
                              ),
                              button: true,
                              enabled: available,
                              label: labelFor(user),
                              child: ListTile(
                                enabled: available,
                                leading: AvatarImage(
                                  imageUrl: avatarUrl(user),
                                  radius: 20,
                                  backgroundColor: context.controlSurface,
                                  fallback: Text(user.initial),
                                  isAgent: user.isAgent,
                                ),
                                title: Text(
                                  labelFor(user),
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                ),
                                subtitle: info == null
                                    ? null
                                    : Text(
                                        info,
                                        maxLines: 1,
                                        overflow: TextOverflow.ellipsis,
                                      ),
                                trailing: Icon(
                                  BuzzIcons.plus,
                                  color: context.colors.onSurfaceVariant,
                                ),
                                onTap: available
                                    ? () => toggleUser(user)
                                    : null,
                              ),
                            ),
                          );
                        },
                      ),
              ),
              if (submitError.value case final error?) ...[
                const SizedBox(height: Grid.xxs),
                Text(
                  error,
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.error,
                  ),
                ),
              ] else if (submitNotice.value case final notice?) ...[
                const SizedBox(height: Grid.xxs),
                Text(
                  notice,
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.onSurfaceVariant,
                  ),
                ),
              ],
              const SizedBox(height: Grid.xxs),
              SizedBox(
                width: double.infinity,
                child: FilledButton(
                  key: const ValueKey('add-channel-members-submit'),
                  onPressed: selectedUsers.value.isEmpty || isSubmitting.value
                      ? null
                      : addSelectedMembers,
                  child: Text(
                    isSubmitting.value
                        ? 'Adding…'
                        : selectedUsers.value.length == 1
                        ? 'Add member'
                        : 'Add ${selectedUsers.value.length} members',
                  ),
                ),
              ),
              const SizedBox(height: Grid.xxs),
            ],
          ),
        ),
      ),
    );
  }
}
