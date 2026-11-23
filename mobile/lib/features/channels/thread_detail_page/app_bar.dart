part of '../thread_detail_page.dart';

/// The conversation's name as its own header shows it.
String _threadConversationName(
  WidgetRef ref,
  Channel channel,
  String? currentPubkey,
) => channel.isDm
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

FrostedAppBar _threadAppBar(
  BuildContext context,
  WidgetRef ref,
  Channel? channel,
  String? currentPubkey,
  _ThreadUpLevel up,
) {
  final channelDisplayName = channel == null
      ? ''
      : _threadConversationName(ref, channel, currentPubkey);
  final upRow = up.row;
  final usesNativeIosGlassBackButton =
      Navigator.canPop(context) &&
      Theme.of(context).platform == TargetPlatform.iOS;

  return FrostedAppBar(
    horizontalInset: Theme.of(context).platform == TargetPlatform.android
        ? Grid.gutter
        : Grid.xxs,
    bottomHeight: conversationNavigationBottomPadding(context) + up.height,
    bottom: upRow == null
        ? null
        : Align(
            alignment: Alignment.topCenter,
            child: SizedBox(height: up.height, child: upRow),
          ),
    alwaysFrosted: true,
    nativeViewSuppressed: messageActionBackdropActive,
    nativeTitle: 'Thread',
    nativeSubtitle: channelDisplayName,
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
      child: const Text('Thread', key: ValueKey('thread-app-bar-title')),
    ),
    titleStyle: channelTitleTextStyle,
  );
}

double _threadHeaderHeight(BuildContext context, _ThreadUpLevel up) =>
    frostedAppBarHeight(
      context,
      bottomHeight: conversationNavigationBottomPadding(context) + up.height,
    );
