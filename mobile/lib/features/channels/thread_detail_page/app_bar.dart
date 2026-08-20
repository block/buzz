part of '../thread_detail_page.dart';

const _threadBackButtonSize = 48.0;

FrostedAppBar _threadAppBar() {
  const timelineAvatarCenter = Grid.gutter + messageAvatarSize / 2;

  return FrostedAppBar(
    horizontalInset: 0,
    leading: SizedBox(
      width: timelineAvatarCenter + _threadBackButtonSize / 2,
      height: _threadBackButtonSize,
      child: Align(
        alignment: Alignment.centerRight,
        child: SizedBox.square(
          dimension: _threadBackButtonSize,
          child: Builder(
            builder: (context) => IconButton(
              key: const ValueKey('thread-app-bar-back'),
              onPressed: () => Navigator.of(context).maybePop(),
              icon: const Icon(LucideIcons.chevronLeft),
              tooltip: 'Back',
            ),
          ),
        ),
      ),
    ),
    title: const Text('Thread'),
    titleStyle: channelTitleTextStyle,
  );
}
