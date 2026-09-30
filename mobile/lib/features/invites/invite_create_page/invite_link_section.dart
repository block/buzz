part of '../invite_create_page.dart';

class _InviteLinkSection extends HookConsumerWidget {
  const _InviteLinkSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final ttlSeconds = useState(defaultCommunityInviteTtlSeconds);
    final maxUses = useState<int?>(null);
    final generationId = useRef(0);
    final invite = useState<MintedCommunityInvite?>(null);
    final isGenerating = useState(true);
    final generationError = useState<String?>(null);

    Future<void> generate() async {
      final requestId = ++generationId.value;
      isGenerating.value = true;
      invite.value = null;
      generationError.value = null;
      try {
        final created = await ref
            .read(communityInviteActionsProvider)
            .mintInvite(ttlSeconds: ttlSeconds.value, maxUses: maxUses.value);
        if (context.mounted && requestId == generationId.value) {
          invite.value = created;
        }
      } catch (error) {
        if (context.mounted && requestId == generationId.value) {
          generationError.value = _inviteErrorMessage(error);
        }
      } finally {
        if (context.mounted && requestId == generationId.value) {
          isGenerating.value = false;
        }
      }
    }

    useEffect(() {
      unawaited(generate());
      return null;
    }, [ttlSeconds.value, maxUses.value]);

    final ttlLabel = communityInviteTtlOptions
        .firstWhere((option) => option.value == ttlSeconds.value)
        .label;
    final maxUsesLabel = communityInviteMaxUseOptions
        .firstWhere((option) => option.value == maxUses.value)
        .label;
    Future<void> share(BuildContext buttonContext) async {
      final url = invite.value?.url;
      if (url == null) return;
      final renderBox = buttonContext.findRenderObject() as RenderBox?;
      final origin = renderBox == null
          ? null
          : renderBox.localToGlobal(Offset.zero) & renderBox.size;
      try {
        await ref.read(shareCommunityInviteProvider)(url, origin);
      } catch (_) {
        if (!context.mounted) return;
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(content: Text('Could not share invite link')),
        );
      }
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Padding(
          padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
          child: Row(
            children: [
              Expanded(
                child: Builder(
                  builder: (buttonContext) => BuzzActionTile(
                    key: const Key('community-invite-share-link'),
                    icon: LucideIcons.share2,
                    label: 'Share',
                    isEnabled: invite.value != null,
                    onTap: () => share(buttonContext),
                  ),
                ),
              ),
              const SizedBox(width: Grid.twelve),
              Expanded(
                child: BuzzActionTile(
                  key: const Key('community-invite-copy-link'),
                  icon: LucideIcons.copy,
                  label: 'Copy',
                  isEnabled: invite.value != null,
                  onTap: () => copyToClipboard(
                    context,
                    invite.value!.url,
                    message: 'Invite link copied',
                  ),
                ),
              ),
            ],
          ),
        ),
        if (generationError.value case final error?) ...[
          const SizedBox(height: Grid.xs),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    error,
                    style: context.textTheme.bodySmall?.copyWith(
                      color: context.colors.error,
                    ),
                  ),
                ),
                TextButton(onPressed: generate, child: const Text('Retry')),
              ],
            ),
          ),
        ],
        const SizedBox(height: Grid.xs),
        AppListCard(
          key: const Key('community-invite-link-settings'),
          dividerIndent: Grid.xs,
          children: [
            AppListRow(
              key: const Key('community-invite-expiry-setting'),
              title: 'Expires after',
              value: ttlLabel,
              trailing: const _InviteRowChevron(),
              onTap: isGenerating.value
                  ? null
                  : () => _showInviteOptionSheet<int>(
                      context: context,
                      title: 'Expires after',
                      value: ttlSeconds.value,
                      options: communityInviteTtlOptions,
                      onSelected: (value) => ttlSeconds.value = value,
                    ),
            ),
            AppListRow(
              key: const Key('community-invite-max-uses-setting'),
              title: 'Limit number of uses',
              value: maxUsesLabel,
              trailing: const _InviteRowChevron(),
              onTap: isGenerating.value
                  ? null
                  : () => _showInviteOptionSheet<int?>(
                      context: context,
                      title: 'Limit number of uses',
                      value: maxUses.value,
                      options: communityInviteMaxUseOptions,
                      onSelected: (value) => maxUses.value = value,
                    ),
            ),
          ],
        ),
      ],
    );
  }
}

void _showInviteOptionSheet<T>({
  required BuildContext context,
  required String title,
  required T value,
  required List<CommunityInviteOption<T>> options,
  required ValueChanged<T> onSelected,
}) {
  showBuzzModalBottomSheet<void>(
    context: context,
    showDragHandle: true,
    builder: (_) => _InviteOptionSheet<T>(
      title: title,
      value: value,
      options: options,
      onSelected: onSelected,
    ),
  );
}

class _InviteOptionSheet<T> extends StatelessWidget {
  const _InviteOptionSheet({
    required this.title,
    required this.value,
    required this.options,
    required this.onSelected,
  });

  final String title;
  final T value;
  final List<CommunityInviteOption<T>> options;
  final ValueChanged<T> onSelected;

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              0,
              Grid.gutter,
              Grid.xxs,
            ),
            child: Text(title, style: context.textTheme.titleMedium),
          ),
          for (final option in options)
            AppListRow(
              key: Key('community-invite-option-${_optionKey(option.label)}'),
              title: option.label,
              trailing: option.value == value
                  ? Icon(
                      LucideIcons.check,
                      size: 18,
                      color: context.colors.primary,
                    )
                  : null,
              onTap: () {
                onSelected(option.value);
                Navigator.of(context).pop();
              },
            ),
          const SizedBox(height: Grid.xxs),
        ],
      ),
    );
  }
}

class _InviteRowChevron extends StatelessWidget {
  const _InviteRowChevron();

  @override
  Widget build(BuildContext context) {
    return Icon(
      LucideIcons.chevronRight,
      size: 18,
      color: context.colors.onSurfaceVariant,
    );
  }
}

String _optionKey(String label) => label
    .toLowerCase()
    .replaceAll(RegExp(r'[^a-z0-9]+'), '-')
    .replaceAll(RegExp(r'^-|-$'), '');
