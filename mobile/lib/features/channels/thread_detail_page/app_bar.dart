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

/// Extra title height the thread header needs for [up]. Only the Flutter
/// title grows with large text; iOS caps its capsule's type within the bar.
double _threadTitleContentHeight(BuildContext context, _ThreadUpLevel? up) =>
    up == null || defaultTargetPlatform == TargetPlatform.iOS
    ? 0
    : _threadUpLevelTitleHeight(context);

FrostedAppBar _threadAppBar(
  BuildContext context,
  WidgetRef ref,
  Channel? channel,
  String? currentPubkey,
  _ThreadUpLevel? up,
) {
  final channelDisplayName = channel == null
      ? ''
      : _threadConversationName(ref, channel, currentPubkey);
  final usesNativeIosGlassBackButton =
      Navigator.canPop(context) &&
      Theme.of(context).platform == TargetPlatform.iOS;
  // With the message menu's backdrop over it, iOS swaps its capsule for the
  // plain title it always has: nothing behind that backdrop can be tapped.
  final flutterTitle = up == null || defaultTargetPlatform == TargetPlatform.iOS
      ? const Text('Thread', key: ValueKey('thread-app-bar-title'))
      : _ThreadUpLevelTitle(up);

  return FrostedAppBar(
    horizontalInset: Theme.of(context).platform == TargetPlatform.android
        ? Grid.gutter
        : Grid.xxs,
    bottomHeight: conversationNavigationBottomPadding(context),
    alwaysFrosted: true,
    nativeViewSuppressed: messageActionBackdropActive,
    nativeTitle: 'Thread',
    // The capsule is the way up: its subtitle names where the thread lives,
    // and the whole capsule goes there. Unresolved, it only names the thread.
    nativeSubtitle: up?.label ?? channelDisplayName,
    nativeSubtitleIcon: up?.icon,
    nativeTitleDisclosure: up?.onPressed != null,
    nativeTitleRefreshing: up != null && up.onPressed == null,
    nativeTitleAccessibilityLabel: up?.semanticLabel,
    // A long parent snippet ends in an ellipsis instead of stretching the
    // capsule across the bar and off its centre.
    nativeTitleMaxWidth: up == null
        ? null
        : conversationTitleMaxWidth(MediaQuery.sizeOf(context).width),
    onNativeTitlePressed: up?.onPressed,
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
    titleContentHeight: _threadTitleContentHeight(context, up),
    title: Padding(
      padding: EdgeInsets.only(
        left: usesNativeIosGlassBackButton
            ? iosGlassChannelHeaderTitleSpacing
            : 0,
      ),
      child: flutterTitle,
    ),
    titleStyle: channelTitleTextStyle,
  );
}

double _threadHeaderHeight(BuildContext context, _ThreadUpLevel? up) =>
    frostedAppBarHeight(
      context,
      bottomHeight: conversationNavigationBottomPadding(context),
      titleContentHeight: _threadTitleContentHeight(context, up),
    );
