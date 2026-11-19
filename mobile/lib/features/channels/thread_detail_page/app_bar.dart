part of '../thread_detail_page.dart';

const _subtitleStyle = TextStyle(fontSize: 12, height: 16 / 12);

double _threadTitleContentHeight(BuildContext context) {
  final scaler = MediaQuery.textScalerOf(context);
  return scaler.scale(channelTitleTextStyle.fontSize!) *
          channelTitleTextStyle.height! +
      scaler.scale(_subtitleStyle.fontSize!) * _subtitleStyle.height!;
}

double _threadAppBarHeight(BuildContext context) => frostedAppBarHeight(
  context,
  titleStyle: channelTitleTextStyle,
  titleContentHeight: _threadTitleContentHeight(context),
  nativeTitleContentHeight: 0,
);

FrostedAppBar _threadAppBar(
  BuildContext context,
  WidgetRef ref,
  Channel? channel,
  String? currentPubkey,
  String rootMessageId,
) {
  final channelDisplayName = channel == null
      ? ''
      : channel.isDm
      ? ref.watch(
          identityNameSourcesProvider.select(
            (names) => resolveDmChannelDisplayLabel(
              channel,
              currentPubkey: currentPubkey,
              names: names,
            ),
          ),
        )
      : channel.name;
  final usesNativeIosGlassBackButton =
      Navigator.canPop(context) &&
      Theme.of(context).platform == TargetPlatform.iOS;

  void openChannel() {
    if (channel == null ||
        !context.mounted ||
        ModalRoute.isCurrentOf(context) != true) {
      return;
    }
    final navigator = Navigator.of(context);
    final routes = ref.read(openChannelRoutesProvider);
    final open = routes.latest(channel.id, navigator: navigator);
    // Popping past another thread would dispose its unsent attachments, so
    // open a fresh channel page above this thread instead.
    if (open != null && !routes.coversThread(open, ModalRoute.of(context)!)) {
      navigator.popUntil((route) => route == open);
      return;
    }
    navigator.push(
      MaterialPageRoute<void>(
        builder: (_) => ChannelDetailPage(
          channel: channel,
          initialMessageId: rootMessageId,
        ),
      ),
    );
  }

  const threadTitle = Text('Thread', key: ValueKey('thread-app-bar-title'));
  final channelLabel = channel == null || channelDisplayName.isEmpty
      ? null
      : channel.isDm
      ? 'Thread, open conversation with $channelDisplayName'
      : 'Thread, open channel $channelDisplayName';

  return FrostedAppBar(
    alwaysFrosted: true,
    nativeViewSuppressed: messageActionBackdropActive,
    nativeTitle: 'Thread',
    nativeSubtitle: channelDisplayName,
    nativeSubtitleNavigationLabel: channelLabel,
    onNativeTitlePressed: channelLabel == null ? null : openChannel,
    leading: usesNativeIosGlassBackButton
        ? IosGlassNavigationButton(
            key: const ValueKey('thread-ios-glass-back'),
            icon: IosGlassNavigationIcon.back,
            semanticLabel: 'Back',
            onPressed: () => Navigator.of(context).maybePop(),
            width: iosGlassChannelHeaderLeadingWidth,
            buttonCenterX: iosGlassChannelHeaderButtonCenterX,
            nativeViewSuppressed: messageActionBackdropActive,
          )
        : null,
    iconColor: context.colors.primary,
    title: Padding(
      padding: EdgeInsets.only(
        left: usesNativeIosGlassBackButton
            ? iosGlassChannelHeaderTitleSpacing
            : 0,
      ),
      child: channelLabel == null
          ? threadTitle
          : Semantics(
              button: true,
              label: channelLabel,
              onTap: openChannel,
              excludeSemantics: true,
              child: InkWell(
                onTap: openChannel,
                child: ConstrainedBox(
                  constraints: const BoxConstraints(minHeight: 48),
                  child: Column(
                    mainAxisAlignment: MainAxisAlignment.center,
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      threadTitle,
                      Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          Flexible(
                            child: Text(
                              channelDisplayName,
                              style: _subtitleStyle.copyWith(
                                color: context.colors.onSurfaceVariant,
                              ),
                            ),
                          ),
                          const SizedBox(width: Grid.quarter),
                          const Icon(BuzzIcons.chevronRight, size: 14),
                        ],
                      ),
                    ],
                  ),
                ),
              ),
            ),
    ),
    titleContentHeight: _threadTitleContentHeight(context),
    nativeTitleContentHeight: 0,
    titleStyle: channelTitleTextStyle,
  );
}
