import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/avatar_image.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import 'channel_management_provider.dart';

/// Debounce before a directory-search query hits the relay.
const _searchDebounce = Duration(milliseconds: 250);

/// Search-and-select sheet for adding people or agents to a channel.
///
/// Unlike the compose bar's `@mention` autocomplete (which only surfaces
/// agents already sharing a channel with you — see `mention_candidates_provider`),
/// this searches the whole community directory, so an agent can be added to
/// a brand-new channel it has never been part of before.
class AddChannelMemberSheet extends HookConsumerWidget {
  final String channelId;
  final Set<String> existingMemberPubkeys;

  const AddChannelMemberSheet({
    super.key,
    required this.channelId,
    required this.existingMemberPubkeys,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final queryController = useTextEditingController();
    final query = useState('');
    final debouncedQuery = useState('');
    final selectedUsers = useState<List<DirectoryUser>>([]);
    final isSubmitting = useState(false);
    final submitError = useState<String?>(null);

    useEffect(() {
      final timer = Timer(_searchDebounce, () {
        debouncedQuery.value = query.value.trim().toLowerCase();
      });
      return timer.cancel;
    }, [query.value]);

    final normalizedQuery = debouncedQuery.value;
    final directoryAsync = normalizedQuery.isEmpty
        ? ref.watch(relayDirectoryUsersProvider)
        : ref.watch(relayDirectorySearchProvider(normalizedQuery));

    final selectedPubkeys = selectedUsers.value
        .map((user) => user.pubkey.toLowerCase())
        .toSet();
    final existingLower = existingMemberPubkeys
        .map((pubkey) => pubkey.toLowerCase())
        .toSet();
    final availableResults =
        directoryAsync.asData?.value
            .where(
              (user) =>
                  !selectedPubkeys.contains(user.pubkey.toLowerCase()) &&
                  !existingLower.contains(user.pubkey.toLowerCase()),
            )
            .toList() ??
        const <DirectoryUser>[];
    final canSubmit = !isSubmitting.value && selectedUsers.value.isNotEmpty;

    Future<void> submit() async {
      if (selectedUsers.value.isEmpty || isSubmitting.value) return;

      isSubmitting.value = true;
      submitError.value = null;
      try {
        final actions = ref.read(channelActionsProvider);
        final agentPubkeys = [
          for (final user in selectedUsers.value)
            if (user.isAgent) user.pubkey,
        ];
        final humanPubkeys = [
          for (final user in selectedUsers.value)
            if (!user.isAgent) user.pubkey,
        ];
        if (agentPubkeys.isNotEmpty) {
          await actions.addMembers(
            channelId: channelId,
            pubkeys: agentPubkeys,
            role: 'bot',
          );
        }
        if (humanPubkeys.isNotEmpty) {
          await actions.addMembers(channelId: channelId, pubkeys: humanPubkeys);
        }
        if (context.mounted) {
          Navigator.of(context).pop();
        }
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
        MediaQuery.viewInsetsOf(context).bottom + Grid.xs,
      ),
      child: SafeArea(
        top: false,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                'Add member',
                style: context.textTheme.titleLarge?.copyWith(
                  fontWeight: FontWeight.w600,
                  letterSpacing: -0.3,
                ),
              ),
              const SizedBox(height: Grid.xs),
              if (selectedUsers.value.isNotEmpty) ...[
                Wrap(
                  spacing: 6,
                  runSpacing: 6,
                  children: [
                    for (final user in selectedUsers.value)
                      _SelectedMemberChip(
                        user: user,
                        enabled: !isSubmitting.value,
                        onDeleted: () {
                          selectedUsers.value = [
                            for (final candidate in selectedUsers.value)
                              if (candidate.pubkey != user.pubkey) candidate,
                          ];
                        },
                      ),
                  ],
                ),
                const SizedBox(height: Grid.xs),
              ],
              TextField(
                key: const Key('add-member-search'),
                controller: queryController,
                autofocus: true,
                autocorrect: false,
                enableSuggestions: false,
                enabled: !isSubmitting.value,
                onChanged: (value) => query.value = value,
                onSubmitted: (_) {
                  if (canSubmit) unawaited(submit());
                },
                decoration: const InputDecoration(
                  hintText: 'Search people or agents',
                  prefixIcon: Icon(LucideIcons.search, size: 18),
                ),
                textInputAction: TextInputAction.done,
              ),
              const SizedBox(height: Grid.xs),
              Builder(
                key: const Key('add-member-results'),
                builder: (context) {
                  if (directoryAsync.isLoading &&
                      directoryAsync.asData == null) {
                    return const SizedBox(
                      height: 200,
                      child: Center(
                        child: BuzzLoadingIndicator(
                          size: 44,
                          semanticLabel: 'Loading people',
                        ),
                      ),
                    );
                  }
                  if (directoryAsync.hasError) {
                    return const SizedBox(
                      height: 120,
                      child: Center(
                        child: Text('Could not load people from this relay.'),
                      ),
                    );
                  }
                  if (availableResults.isEmpty) {
                    return const SizedBox(
                      height: 120,
                      child: Center(
                        child: Text('No matching people or agents.'),
                      ),
                    );
                  }
                  return ListView.separated(
                    physics: const NeverScrollableScrollPhysics(),
                    shrinkWrap: true,
                    itemCount: availableResults.length,
                    separatorBuilder: (_, _) =>
                        const Divider(height: 1, indent: 56),
                    itemBuilder: (context, index) {
                      final user = availableResults[index];
                      return ListTile(
                        key: Key('add-member-result-${user.pubkey}'),
                        contentPadding: const EdgeInsets.symmetric(
                          horizontal: Grid.half,
                        ),
                        leading: AvatarImage(
                          imageUrl: user.avatarUrl,
                          radius: 20,
                          backgroundColor: context.colors.primaryContainer,
                          fallback: Text(
                            user.initial,
                            style: context.textTheme.labelLarge?.copyWith(
                              color: context.colors.onPrimaryContainer,
                              fontWeight: FontWeight.w600,
                            ),
                          ),
                        ),
                        title: Text(
                          user.label,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                        ),
                        subtitle: user.isAgent
                            ? const Text('Agent')
                            : Text(
                                user.secondaryLabel,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                              ),
                        trailing: Icon(
                          LucideIcons.plus,
                          size: 18,
                          color: context.colors.onSurfaceVariant,
                        ),
                        onTap: isSubmitting.value
                            ? null
                            : () {
                                selectedUsers.value = [
                                  ...selectedUsers.value,
                                  user,
                                ];
                                queryController.clear();
                                query.value = '';
                                debouncedQuery.value = '';
                                submitError.value = null;
                              },
                      );
                    },
                  );
                },
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
              const SizedBox(height: Grid.xs),
              SizedBox(
                width: double.infinity,
                child: FilledButton(
                  key: const Key('add-member-submit'),
                  onPressed: canSubmit ? () => unawaited(submit()) : null,
                  child: isSubmitting.value
                      ? const SizedBox.square(
                          dimension: 16,
                          child: BuzzLoadingIndicator(
                            size: 16,
                            semanticLabel: 'Adding',
                          ),
                        )
                      : const Text('Add'),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _SelectedMemberChip extends StatelessWidget {
  final DirectoryUser user;
  final bool enabled;
  final VoidCallback onDeleted;

  const _SelectedMemberChip({
    required this.user,
    required this.enabled,
    required this.onDeleted,
  });

  @override
  Widget build(BuildContext context) {
    return ConstrainedBox(
      key: Key('add-member-selected-${user.pubkey}'),
      constraints: const BoxConstraints(maxWidth: 224),
      child: Material(
        color: context.colors.surfaceContainerHighest,
        shape: const StadiumBorder(),
        clipBehavior: Clip.antiAlias,
        child: Padding(
          padding: const EdgeInsets.symmetric(
            horizontal: Grid.half,
            vertical: 6,
          ),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Flexible(
                child: Text(
                  user.label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: context.textTheme.bodySmall,
                ),
              ),
              const SizedBox(width: 4),
              InkWell(
                onTap: enabled ? onDeleted : null,
                borderRadius: BorderRadius.circular(12),
                child: Icon(
                  LucideIcons.x,
                  size: 14,
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
