import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import 'channel_management_provider.dart';

/// Searchable multi-select used to add people or agents to a channel.
class AddChannelMembersSheet extends HookConsumerWidget {
  const AddChannelMembersSheet({
    super.key,
    required this.channelId,
    required this.existingPubkeys,
  });

  final String channelId;
  final Set<String> existingPubkeys;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final queryController = useTextEditingController();
    final query = useState('');
    final debouncedQuery = useState('');
    final selectedUsers = useState<List<DirectoryUser>>([]);
    final isSubmitting = useState(false);
    final submitError = useState<String?>(null);

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
        await ref
            .read(channelActionsProvider)
            .addMembers(
              channelId: channelId,
              pubkeys: submittedUsers.map((user) => user.pubkey).toList(),
            );
        if (context.mounted) Navigator.of(context).pop(true);
      } on AddMembersException catch (error) {
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
        submitError.value = error.toString();
      } finally {
        isSubmitting.value = false;
      }
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
                  prefixIcon: Icon(LucideIcons.search),
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
                          label: Text(user.label),
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
                            return Semantics(
                              key: ValueKey(
                                'add-channel-member-${user.pubkey}',
                              ),
                              button: true,
                              selected: selected,
                              label: selected
                                  ? '${user.label}, selected'
                                  : user.label,
                              child: ListTile(
                                contentPadding: EdgeInsets.zero,
                                leading: AvatarImage(
                                  imageUrl: user.avatarUrl,
                                  radius: 20,
                                  backgroundColor:
                                      context.colors.primaryContainer,
                                  fallback: Text(user.initial),
                                  isAgent: user.isAgent,
                                ),
                                title: Text(
                                  user.label,
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
                                      ? LucideIcons.circleCheck
                                      : LucideIcons.plus,
                                  color: selected
                                      ? context.colors.primary
                                      : context.colors.onSurfaceVariant,
                                ),
                                onTap: () => toggleUser(user),
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
