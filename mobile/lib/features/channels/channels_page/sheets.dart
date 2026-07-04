part of '../channels_page.dart';

class _QuickActionsSheet extends StatelessWidget {
  const _QuickActionsSheet();

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          Grid.gutter,
          0,
          Grid.gutter,
          Grid.xs,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            ListTile(
              leading: const Icon(LucideIcons.hash),
              title: const Text('Create channel'),
              subtitle: const Text('Start a new stream channel'),
              onTap: () =>
                  Navigator.of(context).pop(_QuickAction.createChannel),
            ),
            ListTile(
              leading: const Icon(LucideIcons.messagesSquare),
              title: const Text('New direct message'),
              subtitle: const Text('Open a DM with one or more people'),
              onTap: () => Navigator.of(context).pop(_QuickAction.newDm),
            ),
          ],
        ),
      ),
    );
  }
}

class _CreateChannelSheet extends HookConsumerWidget {
  final String channelType;

  const _CreateChannelSheet({required this.channelType});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final nameController = useTextEditingController();
    final descriptionController = useTextEditingController();
    final visibility = useState('open');
    final isSubmitting = useState(false);
    final errorMessage = useState<String?>(null);

    Future<void> submit() async {
      final name = nameController.text.trim();
      if (name.isEmpty || isSubmitting.value) {
        return;
      }

      isSubmitting.value = true;
      errorMessage.value = null;
      try {
        final created = await ref
            .read(channelActionsProvider)
            .createChannel(
              name: name,
              channelType: channelType,
              visibility: visibility.value,
              description: descriptionController.text.trim(),
            );
        if (context.mounted) {
          Navigator.of(context).pop(created);
        }
      } catch (error) {
        errorMessage.value = error.toString();
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
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            TextField(
              controller: nameController,
              enabled: !isSubmitting.value,
              decoration: InputDecoration(
                labelText: 'Name',
                hintText: channelType == 'forum'
                    ? 'design-discussions'
                    : 'release-notes',
              ),
              textInputAction: TextInputAction.next,
            ),
            const SizedBox(height: Grid.xxs),
            TextField(
              controller: descriptionController,
              enabled: !isSubmitting.value,
              decoration: const InputDecoration(
                labelText: 'Description',
                hintText: 'What this space is for',
              ),
              minLines: 2,
              maxLines: 3,
            ),
            const SizedBox(height: Grid.xxs),
            SwitchListTile(
              title: const Text('Private'),
              contentPadding: EdgeInsets.zero,
              value: visibility.value == 'private',
              onChanged: isSubmitting.value
                  ? null
                  : (on) => visibility.value = on ? 'private' : 'open',
            ),
            if (errorMessage.value case final error?) ...[
              const SizedBox(height: Grid.xxs),
              Text(
                error,
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.error,
                ),
              ),
            ],
            const SizedBox(height: Grid.xs),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                TextButton(
                  onPressed: isSubmitting.value
                      ? null
                      : () => Navigator.of(context).pop(),
                  child: const Text('Cancel'),
                ),
                const SizedBox(width: Grid.half),
                FilledButton(
                  onPressed: isSubmitting.value ? null : submit,
                  child: Text(isSubmitting.value ? 'Creating…' : 'Create'),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

class _NewDirectMessageSheet extends HookConsumerWidget {
  final String? currentPubkey;

  const _NewDirectMessageSheet({required this.currentPubkey});

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
        debouncedQuery.value = query.value.trim();
      });
      return timer.cancel;
    }, [query.value]);

    final searchFuture = useMemoized(() {
      if (debouncedQuery.value.isEmpty || selectedUsers.value.length >= 8) {
        return Future.value(const <DirectoryUser>[]);
      }
      return ref
          .read(channelActionsProvider)
          .searchUsers(debouncedQuery.value, limit: 8);
    }, [debouncedQuery.value, selectedUsers.value.length]);
    final searchResults = useFuture(searchFuture);

    final selectedPubkeys = selectedUsers.value
        .map((user) => user.pubkey.toLowerCase())
        .toSet();
    final availableResults =
        searchResults.data
            ?.where(
              (user) =>
                  !selectedPubkeys.contains(user.pubkey.toLowerCase()) &&
                  user.pubkey.toLowerCase() != currentPubkey?.toLowerCase(),
            )
            .toList() ??
        const <DirectoryUser>[];
    final canSubmit = !isSubmitting.value && selectedUsers.value.isNotEmpty;

    Future<void> submit() async {
      if (selectedUsers.value.isEmpty || isSubmitting.value) {
        return;
      }

      isSubmitting.value = true;
      submitError.value = null;
      try {
        final channel = await ref
            .read(channelActionsProvider)
            .openDm(
              pubkeys: selectedUsers.value.map((user) => user.pubkey).toList(),
            );
        if (context.mounted) {
          Navigator.of(context).pop(channel);
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
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            TextField(
              controller: queryController,
              decoration: const InputDecoration(
                prefixIcon: Icon(LucideIcons.search),
                hintText: 'Search by name, NIP-05, or pubkey',
              ),
              enabled: !isSubmitting.value,
              onChanged: (value) => query.value = value,
            ),
            if (selectedUsers.value.isNotEmpty) ...[
              const SizedBox(height: Grid.xxs),
              Wrap(
                spacing: Grid.half,
                runSpacing: Grid.half,
                children: [
                  for (final user in selectedUsers.value)
                    InputChip(
                      label: Text(user.label),
                      onDeleted: isSubmitting.value
                          ? null
                          : () {
                              selectedUsers.value = [
                                for (final candidate in selectedUsers.value)
                                  if (candidate.pubkey != user.pubkey)
                                    candidate,
                              ];
                            },
                    ),
                ],
              ),
            ],
            const SizedBox(height: Grid.xs),
            SizedBox(
              height: 280,
              child: Builder(
                builder: (context) {
                  if (selectedUsers.value.length >= 8) {
                    return const Center(
                      child: Text(
                        'Direct messages support up to 9 people including you.',
                      ),
                    );
                  }
                  if (debouncedQuery.value.isEmpty) {
                    return const Center(
                      child: Text(
                        'Search for someone to start a conversation.',
                      ),
                    );
                  }
                  if (searchResults.connectionState ==
                      ConnectionState.waiting) {
                    return const Center(child: CircularProgressIndicator());
                  }
                  if (availableResults.isEmpty) {
                    return const Center(child: Text('No matching users.'));
                  }
                  return ListView(
                    shrinkWrap: true,
                    children: [
                      for (final user in availableResults)
                        ListTile(
                          leading: CircleAvatar(
                            backgroundImage: user.avatarUrl != null
                                ? NetworkImage(user.avatarUrl!)
                                : null,
                            child: user.avatarUrl == null
                                ? Text(user.label.substring(0, 1).toUpperCase())
                                : null,
                          ),
                          title: Text(user.label),
                          subtitle: Text(user.secondaryLabel),
                          onTap: () {
                            selectedUsers.value = [
                              ...selectedUsers.value,
                              user,
                            ];
                            queryController.clear();
                            query.value = '';
                            debouncedQuery.value = '';
                          },
                        ),
                    ],
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
            ],
            const SizedBox(height: Grid.xs),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                TextButton(
                  onPressed: isSubmitting.value
                      ? null
                      : () => Navigator.of(context).pop(),
                  child: const Text('Cancel'),
                ),
                const SizedBox(width: Grid.half),
                FilledButton(
                  onPressed: canSubmit ? submit : null,
                  child: Text(isSubmitting.value ? 'Opening…' : 'Open DM'),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
