part of '../channels_page.dart';

/// "Create your first agent", at the top of DMs where the agent's DM will
/// appear. Opens the setup view once by itself; never blocks the list.
class _FirstAgentRow extends HookConsumerWidget {
  const _FirstAgentRow();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final controller = ref.watch(remoteAgentControllerProvider);
    useListenable(controller);
    final setup = ref.watch(firstAgentSetupProvider);
    final community = ref.watch(activeCommunityProvider).asData?.value;
    final pending = controller.pending;
    final name = (pending?['name'] as String? ?? setup.name).trim();
    final picture = pending?['picture'] as String? ?? setup.picture;
    final started = setup.working || setup.error != null || pending != null;

    void open() {
      if (community == null) return;
      Navigator.of(context).push(
        MaterialPageRoute<void>(
          fullscreenDialog: true,
          builder: (_) => CreateAgentPage(
            controller: controller,
            community: community,
            starter: true,
            onCreated: () {},
          ),
        ),
      );
    }

    // Opens once by itself, only while DMs is on screen, so it never covers
    // another page.
    useEffect(() {
      var active = true;
      bool visible() =>
          active &&
          context.mounted &&
          ModalRoute.of(context)?.isCurrent != false;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!visible()) return;
        claimFirstAgentAutoOpen(controller).then((first) {
          if (first && visible()) open();
        });
      });
      return () => active = false;
    }, const []);

    final subtitle = setup.working
        ? 'Getting ready…'
        : setup.error != null || pending != null
        ? 'Tap to finish setting up'
        : null;
    return InkWell(
      key: const ValueKey('first-agent-row'),
      borderRadius: BorderRadius.circular(Radii.md),
      onTap: open,
      child: Padding(
        padding: const EdgeInsets.symmetric(
          horizontal: _kChannelSectionInset,
          vertical: _kChannelRowVerticalPadding,
        ),
        child: Row(
          children: [
            SizedBox(
              width: _kChannelLeadingWidth,
              child: Align(
                alignment: Alignment.centerLeft,
                child: AvatarImage(
                  imageUrl: started ? picture : null,
                  radius: _kDmAvatarSize / 2,
                  isAgent: true,
                  backgroundColor: context.controlSurface,
                  fallback: Icon(
                    BuzzIcons.plus,
                    size: _kChannelIconSize - 4,
                    color: channelNavigationForeground(context),
                  ),
                ),
              ),
            ),
            const SizedBox(width: _kChannelLabelGap),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    started && name.isNotEmpty
                        ? name
                        : 'Create your first agent',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: navigationChatTitleTextStyle.copyWith(
                      color: channelNavigationForeground(context),
                    ),
                  ),
                  if (subtitle != null)
                    Text(
                      subtitle,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: contentListBodyTextStyle.copyWith(
                        color: navigationMetadataForeground(context),
                      ),
                    ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}
