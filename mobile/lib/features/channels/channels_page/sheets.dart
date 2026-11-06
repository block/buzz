part of '../channels_page.dart';

const int _defaultCreateChannelTtlSeconds = 7 * 24 * 60 * 60;

typedef _CreateChannelMenuOption<T> = BuzzSettingMenuOption<T>;

const _createChannelTtlOptions = [
  _CreateChannelMenuOption(label: '30 minutes', value: 30 * 60),
  _CreateChannelMenuOption(label: '1 hour', value: 60 * 60),
  _CreateChannelMenuOption(label: '6 hours', value: 6 * 60 * 60),
  _CreateChannelMenuOption(label: '12 hours', value: 12 * 60 * 60),
  _CreateChannelMenuOption(label: '1 day', value: 24 * 60 * 60),
  _CreateChannelMenuOption(label: '3 days', value: 3 * 24 * 60 * 60),
  _CreateChannelMenuOption(
    label: '7 days',
    value: _defaultCreateChannelTtlSeconds,
  ),
  _CreateChannelMenuOption(label: '14 days', value: 14 * 24 * 60 * 60),
  _CreateChannelMenuOption(label: '30 days', value: 30 * 24 * 60 * 60),
];

class _CreateChannelSheet extends HookConsumerWidget {
  const _CreateChannelSheet();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final nameController = useTextEditingController();
    final descriptionController = useTextEditingController();
    final visibility = useState('open');
    final temporary = useState(false);
    final ttlSeconds = useState(_defaultCreateChannelTtlSeconds);
    final isSubmitting = useState(false);
    final errorMessage = useState<String?>(null);
    useListenable(nameController);

    final canSubmit =
        nameController.text.trim().isNotEmpty && !isSubmitting.value;

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
              channelType: 'stream',
              visibility: visibility.value,
              description: descriptionController.text.trim(),
              ttlSeconds: temporary.value ? ttlSeconds.value : null,
            );
        if (context.mounted) {
          Navigator.of(context).pop(created);
        }
      } catch (error) {
        if (context.mounted) errorMessage.value = relayErrorMessage(error);
      } finally {
        if (context.mounted) isSubmitting.value = false;
      }
    }

    return _CreationSheetLayout(
      title: 'New channel',
      actionKey: const Key('create-channel-submit'),
      actionLabel: 'Create',
      busy: isSubmitting.value,
      onSubmit: canSubmit ? submit : null,
      child: Padding(
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
                const ExcludeSemantics(
                  child: _CreateChannelFieldLabel(label: 'Name'),
                ),
                const SizedBox(height: Grid.xxs),
                _CreateChannelFieldShell(
                  label: 'Name',
                  child: TextField(
                    key: const Key('create-channel-name'),
                    controller: nameController,
                    enabled: !isSubmitting.value,
                    autofocus: true,
                    autocorrect: false,
                    enableSuggestions: false,
                    textCapitalization: TextCapitalization.none,
                    decoration: InputDecoration(
                      hintText: 'release-notes',
                      border: InputBorder.none,
                      enabledBorder: InputBorder.none,
                      focusedBorder: InputBorder.none,
                      contentPadding: const EdgeInsets.symmetric(
                        horizontal: Grid.twelve,
                        vertical: Grid.xs,
                      ),
                    ),
                    textInputAction: TextInputAction.done,
                    onSubmitted: (_) {
                      if (canSubmit) {
                        unawaited(submit());
                      }
                    },
                  ),
                ),
                const SizedBox(height: Grid.xs),
                const ExcludeSemantics(
                  child: _CreateChannelFieldLabel(
                    label: 'Description',
                    optional: true,
                  ),
                ),
                const SizedBox(height: Grid.xxs),
                _CreateChannelFieldShell(
                  label: 'Description, optional',
                  child: TextField(
                    key: const Key('create-channel-description'),
                    controller: descriptionController,
                    enabled: !isSubmitting.value,
                    decoration: InputDecoration(
                      hintText: 'What this channel is for',
                      border: InputBorder.none,
                      enabledBorder: InputBorder.none,
                      focusedBorder: InputBorder.none,
                      contentPadding: const EdgeInsets.symmetric(
                        horizontal: Grid.twelve,
                        vertical: Grid.xs,
                      ),
                    ),
                    minLines: 2,
                    maxLines: 3,
                    textInputAction: TextInputAction.done,
                    onSubmitted: (_) {
                      if (canSubmit) {
                        unawaited(submit());
                      }
                    },
                  ),
                ),
                const SizedBox(height: Grid.sm),
                SheetActionSection(
                  dividerIndent: Grid.xs,
                  children: [
                    BuzzSettingMenu<bool>(
                      controlKey: const Key('create-channel-type'),
                      grouped: true,
                      preserveTextInput: true,
                      trailingIcon: Icons.unfold_more,
                      valueLabel: temporary.value ? 'Temporary' : 'Ongoing',
                      enabled: !isSubmitting.value,
                      label: 'Channel type',
                      onSelected: (value) => temporary.value = value,
                      options: const [
                        _CreateChannelMenuOption(
                          key: Key('create-channel-type-ongoing'),
                          label: 'Ongoing',
                          value: false,
                        ),
                        _CreateChannelMenuOption(
                          key: Key('create-channel-type-temporary'),
                          label: 'Temporary',
                          value: true,
                        ),
                      ],
                      value: temporary.value,
                    ),
                    if (temporary.value) ...[
                      BuzzSettingMenu<int>(
                        grouped: true,
                        preserveTextInput: true,
                        trailingIcon: Icons.unfold_more,
                        controlKey: const Key('create-channel-ttl'),
                        enabled: !isSubmitting.value,
                        label: 'Expires after',
                        onSelected: (value) => ttlSeconds.value = value,
                        options: _createChannelTtlOptions,
                        value: ttlSeconds.value,
                        valueLabel: _createChannelTtlOptions
                            .firstWhere(
                              (option) => option.value == ttlSeconds.value,
                            )
                            .label,
                      ),
                    ],
                    BuzzSettingMenu<String>(
                      controlKey: const Key('create-channel-visibility'),
                      grouped: true,
                      preserveTextInput: true,
                      trailingIcon: Icons.unfold_more,
                      valueLabel: visibility.value == 'open'
                          ? 'Public'
                          : 'Private',
                      enabled: !isSubmitting.value,
                      label: 'Visibility',
                      onSelected: (value) => visibility.value = value,
                      options: const [
                        _CreateChannelMenuOption(
                          key: Key('create-channel-visibility-public'),
                          label: 'Public',
                          value: 'open',
                        ),
                        _CreateChannelMenuOption(
                          key: Key('create-channel-visibility-private'),
                          label: 'Private',
                          value: 'private',
                        ),
                      ],
                      value: visibility.value,
                    ),
                  ],
                ),
                if (errorMessage.value case final error?) ...[
                  const SizedBox(height: Grid.xs),
                  Semantics(
                    liveRegion: true,
                    child: Text(
                      error,
                      style: context.textTheme.bodySmall?.copyWith(
                        color: context.colors.error,
                      ),
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _CreateChannelFieldLabel extends StatelessWidget {
  final String label;
  final bool optional;

  const _CreateChannelFieldLabel({required this.label, this.optional = false});

  @override
  Widget build(BuildContext context) {
    return Text.rich(
      TextSpan(
        children: [
          TextSpan(text: label),
          if (optional)
            TextSpan(
              text: '  Optional',
              style: context.textTheme.labelSmall?.copyWith(
                color: context.colors.onSurfaceVariant.withValues(alpha: 0.6),
                fontWeight: FontWeight.w400,
              ),
            ),
        ],
      ),
      style: context.textTheme.labelLarge?.copyWith(
        color: context.colors.onSurface,
        fontWeight: FontWeight.w600,
      ),
    );
  }
}

class _CreateChannelFieldShell extends StatelessWidget {
  final Widget child;

  final String label;

  const _CreateChannelFieldShell({required this.child, required this.label});

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        color: context.colors.surfaceContainerHighest,
        borderRadius: BorderRadius.circular(Radii.container),
      ),
      child: Semantics(label: label, child: child),
    );
  }
}

class _NewDirectMessageSheet extends HookConsumerWidget {
  final String? currentPubkey;

  final bool nativePresentation;
  const _NewDirectMessageSheet({
    required this.currentPubkey,
    this.nativePresentation = false,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final queryController = useTextEditingController();
    final queryFocusNode = useFocusNode();
    final query = useState('');
    // A new opening after each pick, as the mention chooser starts one per
    // `@`: shown rows keep their places only within one query and opening.
    final opening = useState(0);
    final selectedUsers = useState<List<MentionCandidate>>([]);
    final isSubmitting = useState(false);
    final submitError = useState<String?>(null);
    final completedChannel = useState<Channel?>(null);

    // The same candidates, ranking, debounce and search cache as mention
    // autocomplete, so a name finds the same people and agents in both.
    // A Space never picks here, so a search ignores its edge spaces where a
    // composer query would end at one.
    final search = query.value.trim();
    final ranked = ref.watch(
      recipientCandidatesProvider((
        query: search,
        historyKey: newDmMentionHistoryKey,
        unsharedAgents: false,
      )),
    );
    final searching = ref.watch(recipientSearchPendingProvider(search));
    final searchFailed = ref.watch(recipientSearchFailedProvider(search));
    final selectedPubkeys = {
      for (final user in selectedUsers.value) user.pubkey,
    };
    final shown = useStableMentionRows(
      [
        for (final candidate in ranked)
          if (!selectedPubkeys.contains(candidate.pubkey)) candidate,
      ],
      query: search,
      opening: opening.value,
      pending: searching,
    );
    final availableResults = shown.rows;
    // Shown and chosen recipients are named together on every build, so a
    // pick stays told apart from a same-name result of a later search, and
    // loading their profiles tracks them for verified names.
    final recipients = [...availableResults, ...selectedUsers.value];
    final names = watchIdentityNames(
      ref,
      [for (final user in recipients) user.pubkey],
      agentPubkeys: {
        for (final user in recipients)
          if (user.isAgent) user.pubkey,
      },
      fallbackNames: {
        for (final user in recipients) user.pubkey: user.pickerLabel,
      },
    );
    String labelFor(MentionCandidate user) => names.labelFor(user.pubkey);
    final userCache = ref.watch(userCacheProvider);
    String? detail(MentionCandidate user) =>
        recipientRowDetail(user, currentPubkey, userCache);
    void select(MentionCandidate user) {
      selectedUsers.value = [...selectedUsers.value, user];
      opening.value++;
      ref
          .read(mentionHistoryProvider.notifier)
          .remember(newDmMentionHistoryKey, user.pubkey);
    }

    void retry() => ref.invalidate(mentionUserSearchProvider(search));
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
              waitForChannelList: false,
            );
        if (context.mounted) {
          if (nativePresentation) {
            completedChannel.value = channel;
          } else {
            Navigator.of(context).pop(channel);
          }
        }
      } catch (error) {
        if (context.mounted) submitError.value = error.toString();
      } finally {
        if (context.mounted) isSubmitting.value = false;
      }
    }

    if (nativePresentation) {
      final auth = ref.watch(mediaGetAuthServiceProvider);
      final client = ref.watch(mediaHttpClientProvider);
      final colors = context.colors;
      String? avatarUrl(MentionCandidate user) =>
          user.avatarUrl ?? userCache[user.pubkey]?.avatarUrl;
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
      };
      MentionCandidate? find(String pubkey) => [
        ...selectedUsers.value,
        ...availableResults,
      ].where((user) => user.pubkey == pubkey).firstOrNull;
      return IosAddMembersSheet(
        onAvatar: (pubkey, key) async {
          final user = find(pubkey);
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
          'title': 'New DM',
          'confirmWithCheckmark': true,
          'clearQueryOnPick': true,
          'emptyQueryHint': 'Type a name to find people or agents.',
          'selectionLimit': 8,
          'completed': completedChannel.value != null,
          'query': query.value,
          'loading': searching,
          'loadError': searchFailed,
          'error': submitError.value,
          'submitting': isSubmitting.value,
          'selected': [for (final user in selectedUsers.value) row(user)],
          'users': [for (final user in availableResults) row(user)],
        },
        onQuery: (value) {
          if (!isSubmitting.value) query.value = value;
        },
        onToggle: (pubkey) {
          if (isSubmitting.value) return;
          if (selectedPubkeys.contains(pubkey)) {
            selectedUsers.value = selectedUsers.value
                .where((user) => user.pubkey != pubkey)
                .toList();
            opening.value++;
            return;
          }
          final user = find(pubkey);
          if (user == null ||
              shown.unavailable.contains(pubkey) ||
              selectedUsers.value.length >= 8) {
            return;
          }
          select(user);
        },
        onSubmit: () => unawaited(submit()),
        onRetry: retry,
        onClose: () => Navigator.of(context).pop(completedChannel.value),
      );
    }

    return _CreationSheetLayout(
      title: 'New DM',
      actionKey: const Key('new-dm-submit'),
      actionLabel: 'Done',
      busy: isSubmitting.value,
      onSubmit: canSubmit ? submit : null,
      child: Padding(
        padding: EdgeInsets.fromLTRB(
          Grid.gutter,
          0,
          Grid.gutter,
          MediaQuery.viewInsetsOf(context).bottom + Grid.xs,
        ),
        child: SafeArea(
          top: false,
          // The sheet bounds this viewport even with the keyboard visible.
          // Keep rows as slivers so search and chip updates do not lay out the
          // entire directory. The field still scrolls with long chip selections.
          child: CustomScrollView(
            shrinkWrap: true,
            slivers: [
              SliverToBoxAdapter(
                child: GestureDetector(
                  behavior: HitTestBehavior.translucent,
                  onTap: isSubmitting.value
                      ? null
                      : queryFocusNode.requestFocus,
                  child: SizedBox(
                    width: double.infinity,
                    child: ConstrainedBox(
                      key: const Key('new-dm-recipient-field'),
                      constraints: const BoxConstraints(minHeight: Grid.xl),
                      child: DecoratedBox(
                        decoration: BoxDecoration(
                          color: context.colors.surface,
                          border: Border.all(
                            color: context.colors.outlineVariant,
                          ),
                          borderRadius: BorderRadius.circular(Radii.container),
                        ),
                        child: Padding(
                          padding: const EdgeInsets.symmetric(
                            horizontal: Grid.twelve,
                            vertical: Grid.xxs,
                          ),
                          child: LayoutBuilder(
                            builder: (context, constraints) {
                              final hasSelectedUsers =
                                  selectedUsers.value.isNotEmpty;
                              final searchFieldWidth = hasSelectedUsers
                                  ? 96.0
                                  : (constraints.maxWidth - 32).clamp(
                                      96.0,
                                      constraints.maxWidth,
                                    );

                              return Wrap(
                                key: const Key('new-dm-recipient-wrap'),
                                spacing: 6,
                                runSpacing: 6,
                                crossAxisAlignment: WrapCrossAlignment.center,
                                children: [
                                  Padding(
                                    padding: const EdgeInsets.symmetric(
                                      vertical: Grid.half,
                                    ),
                                    child: Text(
                                      'To:',
                                      style: context.textTheme.bodyLarge
                                          ?.copyWith(
                                            fontWeight: FontWeight.w600,
                                          ),
                                    ),
                                  ),
                                  for (final user in selectedUsers.value)
                                    _SelectedDmRecipientChip(
                                      user: user,
                                      label: labelFor(user),
                                      enabled: !isSubmitting.value,
                                      onDeleted: () {
                                        selectedUsers.value = [
                                          for (final candidate
                                              in selectedUsers.value)
                                            if (candidate.pubkey != user.pubkey)
                                              candidate,
                                        ];
                                        opening.value++;
                                        queryFocusNode.requestFocus();
                                      },
                                    ),
                                  SizedBox(
                                    width: searchFieldWidth,
                                    child: TextField(
                                      key: const Key('new-dm-search'),
                                      controller: queryController,
                                      focusNode: queryFocusNode,
                                      autofocus: true,
                                      autocorrect: false,
                                      enableSuggestions: false,
                                      enabled: !isSubmitting.value,
                                      onChanged: (value) => query.value = value,
                                      onSubmitted: (_) {
                                        if (canSubmit) {
                                          unawaited(submit());
                                        }
                                      },
                                      decoration: InputDecoration(
                                        hintText: hasSelectedUsers
                                            ? null
                                            : 'Search for people or agents',
                                        border: InputBorder.none,
                                        enabledBorder: InputBorder.none,
                                        focusedBorder: InputBorder.none,
                                        isDense: true,
                                        contentPadding:
                                            const EdgeInsets.symmetric(
                                              vertical: Grid.half,
                                            ),
                                        suffixIcon: isSubmitting.value
                                            ? const Padding(
                                                padding: EdgeInsets.all(
                                                  Grid.half,
                                                ),
                                                child: SizedBox.square(
                                                  dimension: 16,
                                                  child: BuzzLoadingIndicator(
                                                    size: 16,
                                                    semanticLabel:
                                                        'Creating conversation',
                                                  ),
                                                ),
                                              )
                                            : null,
                                        suffixIconConstraints:
                                            const BoxConstraints(
                                              minHeight: 32,
                                              minWidth: 32,
                                            ),
                                      ),
                                      textInputAction: TextInputAction.done,
                                    ),
                                  ),
                                ],
                              );
                            },
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
              const SliverToBoxAdapter(child: SizedBox(height: Grid.xs)),
              Builder(
                key: const Key('new-dm-results'),
                builder: (context) {
                  if (selectedUsers.value.length >= 8) {
                    return const SliverToBoxAdapter(
                      child: SizedBox(
                        height: 96,
                        child: Center(
                          child: Text(
                            'DMs support up to nine people, including you.',
                          ),
                        ),
                      ),
                    );
                  }
                  if (availableResults.isEmpty && !searchFailed) {
                    return SliverToBoxAdapter(
                      child: SizedBox(
                        height: 96,
                        child: Center(
                          child: searching
                              ? const BuzzLoadingIndicator(
                                  size: 32,
                                  semanticLabel: 'Searching people',
                                )
                              : Text(
                                  query.value.trim().isEmpty
                                      ? 'Type a name to find people or agents.'
                                      : 'No matching people or agents.',
                                ),
                        ),
                      ),
                    );
                  }
                  return SliverList.builder(
                    itemCount: availableResults.length + (searchFailed ? 1 : 0),
                    itemBuilder: (context, index) {
                      if (index == availableResults.length) {
                        return Material(
                          type: MaterialType.transparency,
                          child: ListTile(
                            key: const Key('new-dm-search-error'),
                            title: Text(
                              'Could not search community people.',
                              style: context.textTheme.bodySmall?.copyWith(
                                color: context.colors.error,
                              ),
                            ),
                            trailing: TextButton(
                              onPressed: retry,
                              child: const Text('Retry'),
                            ),
                          ),
                        );
                      }
                      final user = availableResults[index];
                      final available = !shown.unavailable.contains(
                        user.pubkey,
                      );
                      final info = detail(user);
                      return AppListCardItem(
                        index: index,
                        itemCount: availableResults.length,
                        dividerIndent: Grid.xs + 40 + Grid.xs,
                        child: ListTile(
                          key: Key('new-dm-person-${user.pubkey}'),
                          enabled: available,
                          leading: AvatarImage(
                            imageUrl:
                                user.avatarUrl ??
                                userCache[user.pubkey]?.avatarUrl,
                            radius: 20,
                            backgroundColor: context.controlSurface,
                            fallback: Text(
                              user.initial,
                              style: context.textTheme.labelLarge?.copyWith(
                                color: context.colors.onPrimaryContainer,
                                fontWeight: FontWeight.w600,
                              ),
                            ),
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
                          onTap: isSubmitting.value || !available
                              ? null
                              : () {
                                  select(user);
                                  queryController.clear();
                                  query.value = '';
                                  submitError.value = null;
                                  queryFocusNode.requestFocus();
                                },
                        ),
                      );
                    },
                  );
                },
              ),
              if (submitError.value case final error?) ...[
                const SliverToBoxAdapter(child: SizedBox(height: Grid.xxs)),
                SliverToBoxAdapter(
                  child: Text(
                    error,
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.error,
                    ),
                  ),
                ),
              ],
            ],
          ),
        ),
      ),
    );
  }
}

class _SelectedDmRecipientChip extends StatelessWidget {
  final MentionCandidate user;
  final String label;
  final bool enabled;
  final VoidCallback onDeleted;

  const _SelectedDmRecipientChip({
    required this.user,
    required this.label,
    required this.enabled,
    required this.onDeleted,
  });

  @override
  Widget build(BuildContext context) {
    final foreground = context.colors.onSurfaceVariant;

    return ConstrainedBox(
      key: Key('new-dm-selected-${user.pubkey}'),
      constraints: const BoxConstraints(maxWidth: 224),
      child: SizedBox(
        height: 40,
        child: Material(
          color: context.colors.surfaceContainerHighest,
          shape: const StadiumBorder(),
          clipBehavior: Clip.antiAlias,
          child: Semantics(
            button: true,
            enabled: enabled,
            excludeSemantics: true,
            label: 'Remove $label',
            child: InkWell(
              customBorder: const StadiumBorder(),
              onTap: enabled ? onDeleted : null,
              child: Padding(
                padding: const EdgeInsets.only(
                  left: Grid.half,
                  right: Grid.twelve,
                ),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    AvatarImage(
                      imageUrl: user.avatarUrl,
                      radius: 16,
                      backgroundColor: context.controlSurface,
                      fallback: Text(
                        user.initial,
                        style: context.textTheme.labelMedium?.copyWith(
                          color: context.colors.onPrimaryContainer,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                      isAgent: user.isAgent,
                    ),
                    const SizedBox(width: Grid.xxs),
                    Flexible(
                      child: Text(
                        label,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: context.textTheme.bodyLarge?.copyWith(
                          color: foreground,
                          fontWeight: FontWeight.w500,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _CreationSheetLayout extends StatelessWidget {
  const _CreationSheetLayout({
    required this.title,
    required this.actionKey,
    required this.actionLabel,
    required this.busy,
    required this.onSubmit,
    required this.child,
  });

  final String title;
  final Key actionKey;
  final String actionLabel;
  final bool busy;
  final VoidCallback? onSubmit;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final close = busy ? null : () => Navigator.of(context).pop();
    final ios = defaultTargetPlatform == TargetPlatform.iOS;
    return PopScope(
      canPop: !busy,
      child: BuzzTitledSheetLayout(
        title: title,
        leading: ios
            ? IosGlassNavigationButton(
                icon: IosGlassNavigationIcon.close,
                semanticLabel: 'Close',
                onPressed: close,
              )
            : IconButton(
                tooltip: 'Close',
                onPressed: close,
                icon: const Icon(BuzzIcons.x),
              ),
        trailing: actionLabel == 'Done'
            ? (ios
                  ? IosGlassNavigationButton(
                      key: actionKey,
                      icon: IosGlassNavigationIcon.check,
                      semanticLabel: 'Done',
                      onPressed: onSubmit,
                      isBusy: busy,
                    )
                  : IconButton(
                      key: actionKey,
                      tooltip: 'Done',
                      onPressed: onSubmit,
                      icon: busy
                          ? const SizedBox.square(
                              dimension: 20,
                              child: CircularProgressIndicator(strokeWidth: 2),
                            )
                          : const Icon(BuzzIcons.check),
                    ))
            : ios
            ? IosGlassNavigationAction(
                width: 80,
                key: actionKey,
                label: actionLabel,
                onPressed: onSubmit,
                isBusy: busy,
              )
            : TextButton(
                key: actionKey,
                onPressed: onSubmit,
                child: busy
                    ? const SizedBox.square(
                        dimension: 20,
                        child: CircularProgressIndicator(
                          strokeWidth: 2,
                          semanticsLabel: 'Creating',
                        ),
                      )
                    : Text(actionLabel),
              ),
        child: child,
      ),
    );
  }
}
