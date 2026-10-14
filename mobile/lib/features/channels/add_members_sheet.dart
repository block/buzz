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
import '../../shared/identity_names/identity_names_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/relay/relay.dart';
import 'channel_management_provider.dart';
import 'ios_add_members_sheet.dart';
import '../../shared/widgets/modal_presentation.dart';

bool _nativePickerOpen = false;

/// Opens the platform member picker, retaining the Android sheet presentation.
Future<bool?> showAddChannelMembersSheet({
  required BuildContext context,
  required String channelId,
  required Set<String> existingPubkeys,
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
    ),
  );
}

/// Searchable multi-select used to add people or agents to a channel.
class AddChannelMembersSheet extends HookConsumerWidget {
  const AddChannelMembersSheet({
    super.key,
    required this.channelId,
    required this.existingPubkeys,
    this.nativePresentation = false,
  });

  final String channelId;
  final Set<String> existingPubkeys;
  final bool nativePresentation;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final queryController = useTextEditingController();
    final query = useState('');
    final debouncedQuery = useState('');
    final selectedUsers = useState<List<DirectoryUser>>([]);
    final isSubmitting = useState(false);
    final submitError = useState<String?>(null);
    final actions = useMemoized(() => ref.read(channelActionsProvider));

    useEffect(() {
      final timer = Timer(const Duration(milliseconds: 250), () {
        debouncedQuery.value = query.value.trim().toLowerCase();
      });
      return timer.cancel;
    }, [query.value]);

    final normalizedQuery = debouncedQuery.value;
    final directoryAsync = normalizedQuery.isEmpty
        ? ref.watch(relayDirectoryUsersProvider)
        : ref.watch(relayDirectorySearchProvider(normalizedQuery));
    final normalizedExisting = existingPubkeys
        .map((pubkey) => pubkey.toLowerCase())
        .toSet();
    final selectedPubkeys = selectedUsers.value
        .map((user) => user.pubkey.toLowerCase())
        .toSet();
    final successfulPubkeys = useState<Set<String>>(<String>{});
    final excludedPubkeys = {...normalizedExisting, ...successfulPubkeys.value};
    final availableUsers =
        directoryAsync.asData?.value
            .where(
              (user) => !excludedPubkeys.contains(user.pubkey.toLowerCase()),
            )
            .toList() ??
        const <DirectoryUser>[];
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
        for (final user in choices) user.pubkey: user.label,
      },
      shown: [for (final user in choices) user.pubkey],
    );
    String labelFor(DirectoryUser user) => names.labelFor(user.pubkey);

    void toggleUser(DirectoryUser user) {
      if (isSubmitting.value) return;
      final normalized = user.pubkey.toLowerCase();
      selectedUsers.value = selectedPubkeys.contains(normalized)
          ? [
              for (final selected in selectedUsers.value)
                if (selected.pubkey.toLowerCase() != normalized) selected,
            ]
          : [...selectedUsers.value, user];
      submitError.value = null;
    }

    Future<void> addSelectedMembers() async {
      if (selectedUsers.value.isEmpty || isSubmitting.value) return;
      final submittedUsers = List<DirectoryUser>.of(selectedUsers.value);
      isSubmitting.value = true;
      submitError.value = null;
      try {
        await actions.addMembers(
          channelId: channelId,
          pubkeys: submittedUsers.map((user) => user.pubkey).toList(),
        );
        if (context.mounted) Navigator.of(context).pop(true);
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
        submitError.value = error.toString();
      } finally {
        if (context.mounted) isSubmitting.value = false;
      }
    }

    if (nativePresentation) {
      final profiles = ref.watch(userCacheProvider);
      final auth = ref.watch(mediaGetAuthServiceProvider);
      final client = ref.watch(mediaHttpClientProvider);
      final colors = Theme.of(context).colorScheme;
      String? avatarUrl(DirectoryUser user) =>
          profiles[user.pubkey]?.avatarUrl ?? user.avatarUrl;
      String avatarKey(DirectoryUser user) => jsonEncode([
        user.pubkey,
        avatarUrl(user),
        user.initial,
        user.isAgent,
        colors.primaryContainer.toARGB32(),
        colors.onPrimaryContainer.toARGB32(),
      ]);
      Map<String, Object?> row(DirectoryUser user) => {
        'pubkey': user.pubkey,
        'name': labelFor(user),
        'detail': user.secondaryLabel,
        'agent': user.isAgent,
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
            networkImage: (url) =>
                MediaImageProvider(url: url, auth: auth, client: client),
          );
        },
        state: {
          'query': query.value,
          'loading':
              directoryAsync.isLoading ||
              query.value.trim().toLowerCase() != normalizedQuery,
          'loadError': directoryAsync.hasError,
          'error': submitError.value,
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
        onRetry: () {
          if (normalizedQuery.isEmpty) {
            ref.invalidate(relayDirectoryUsersProvider);
          } else {
            ref.invalidate(relayDirectorySearchProvider(normalizedQuery));
          }
        },
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
                child: directoryAsync.when(
                  data: (_) => availableUsers.isEmpty
                      ? Center(
                          child: Text(
                            normalizedQuery.isEmpty
                                ? 'Everyone available is already in this channel.'
                                : 'No matching people or agents.',
                            textAlign: TextAlign.center,
                          ),
                        )
                      : ListView.builder(
                          key: const ValueKey('add-channel-members-results'),
                          itemCount: availableUsers.length,
                          itemBuilder: (context, index) {
                            final user = availableUsers[index];
                            final selected = selectedPubkeys.contains(
                              user.pubkey.toLowerCase(),
                            );
                            return AppListCardItem(
                              index: index,
                              itemCount: availableUsers.length,
                              dividerIndent: Grid.xs + 40 + Grid.xs,
                              child: Semantics(
                                key: ValueKey(
                                  'add-channel-member-${user.pubkey}',
                                ),
                                button: true,
                                selected: selected,
                                label: selected
                                    ? '${labelFor(user)}, selected'
                                    : labelFor(user),
                                child: ListTile(
                                  leading: AvatarImage(
                                    imageUrl: user.avatarUrl,
                                    radius: 20,
                                    backgroundColor:
                                        context.colors.primaryContainer,
                                    fallback: Text(user.initial),
                                    isAgent: user.isAgent,
                                  ),
                                  title: Text(
                                    labelFor(user),
                                    maxLines: 1,
                                    overflow: TextOverflow.ellipsis,
                                  ),
                                  subtitle: Text(
                                    user.secondaryLabel,
                                    maxLines: 1,
                                    overflow: TextOverflow.ellipsis,
                                  ),
                                  trailing: Icon(
                                    selected
                                        ? BuzzIcons.circleCheck
                                        : BuzzIcons.plus,
                                    color: selected
                                        ? context.colors.primary
                                        : context.colors.onSurfaceVariant,
                                  ),
                                  onTap: () => toggleUser(user),
                                ),
                              ),
                            );
                          },
                        ),
                  loading: () => const Center(
                    child: BuzzLoadingIndicator(
                      size: 44,
                      semanticLabel: 'Loading people and agents',
                    ),
                  ),
                  error: (error, _) => Center(
                    child: Text(
                      'Couldn’t load people or agents. Try again.',
                      style: context.textTheme.bodyMedium?.copyWith(
                        color: context.colors.error,
                      ),
                      textAlign: TextAlign.center,
                    ),
                  ),
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
