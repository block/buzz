part of '../channel_detail_page.dart';

class _ActivityMembersSheet extends HookConsumerWidget {
  const _ActivityMembersSheet({required this.pubkeys, required this.channelId});

  final List<String> pubkeys;
  final String channelId;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final cache = ref.watch(userCacheProvider);
    final names = ref.watch(channelIdentityNamesProvider(channelId));
    useEffect(() {
      ref.read(userCacheProvider.notifier).preload(pubkeys);
      return null;
    }, [pubkeys]);
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxHeight: 400),
        child: ListView(
          shrinkWrap: true,
          padding: EdgeInsets.only(
            bottom: Grid.md + MediaQuery.viewPaddingOf(context).bottom,
          ),
          children: [
            SheetActionSection(
              children: [
                for (final pubkey in pubkeys)
                  ListTile(
                    key: ValueKey('activity-member-$pubkey'),
                    leading: SmallAvatar(
                      pubkey: pubkey,
                      userCache: cache,
                      size: 40,
                    ),
                    title: Text(names.labelFor(pubkey)),
                    onTap: () => showUserProfileSheet(
                      context,
                      pubkey,
                      names: channelIdentityNamesProvider(channelId),
                    ),
                  ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
