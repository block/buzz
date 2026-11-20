part of '../channel_detail_page.dart';

class _ActivityMembersSheet extends ConsumerWidget {
  const _ActivityMembersSheet({required this.pubkeys, required this.channelId});

  final List<String> pubkeys;
  final String channelId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return Padding(
      padding: EdgeInsets.fromLTRB(
        Grid.gutter,
        Grid.xxs,
        Grid.gutter,
        Grid.md + MediaQuery.viewPaddingOf(context).bottom,
      ),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxHeight: 400),
        child: Material(
          color: context.colors.surfaceContainerHighest,
          borderRadius: BorderRadius.circular(Radii.container),
          clipBehavior: Clip.antiAlias,
          child: ListTileTheme.merge(
            contentPadding: const EdgeInsets.symmetric(horizontal: Grid.xs),
            minLeadingWidth: 22,
            horizontalTitleGap: Grid.xs,
            child: ListView.separated(
              key: const ValueKey('activity-members-list'),
              shrinkWrap: true,
              scrollCacheExtent: const ScrollCacheExtent.pixels(0),
              padding: const EdgeInsets.symmetric(horizontal: Grid.xs),
              itemCount: pubkeys.length,
              separatorBuilder: (context, index) => Divider(
                height: 1,
                thickness: 1,
                indent: Grid.xs * 2 + 22,
                endIndent: Grid.xs,
                color: context.colors.onSurface.withValues(alpha: 0.07),
              ),
              itemBuilder: (context, index) => _ActivityMemberRow(
                key: ValueKey('activity-member-${pubkeys[index]}'),
                pubkey: pubkeys[index],
                channelId: channelId,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _ActivityMemberRow extends HookConsumerWidget {
  const _ActivityMemberRow({
    super.key,
    required this.pubkey,
    required this.channelId,
  });
  final String pubkey;
  final String channelId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final cache = ref.watch(userCacheProvider);
    final names = ref.watch(channelIdentityNamesProvider(channelId));
    useEffect(() {
      ref.read(userCacheProvider.notifier).preload([pubkey]);
      return null;
    }, [pubkey]);
    return ListTile(
      leading: SmallAvatar(pubkey: pubkey, userCache: cache, size: 40),
      title: Text(names.labelFor(pubkey)),
      onTap: () => showUserProfileSheet(
        context,
        pubkey,
        names: channelIdentityNamesProvider(channelId),
      ),
    );
  }
}
