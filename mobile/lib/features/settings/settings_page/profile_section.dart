part of '../settings_page.dart';

class _ProfileSection extends StatelessWidget {
  const _ProfileSection({
    required this.profileEditPageBuilder,
    this.onEditDisplayName,
    this.onEditProfileDescription,
  });

  final WidgetBuilder profileEditPageBuilder;
  final Future<void> Function(BuildContext context)? onEditDisplayName;
  final Future<void> Function(BuildContext context)? onEditProfileDescription;

  @override
  Widget build(BuildContext context) => AppListCard(
    key: const ValueKey('edit-profile-options'),
    dividerIndent: Grid.xs,
    verticalPadding: Grid.twelve,
    children: [
      AppListRow(
        key: const ValueKey('edit-profile-display-name'),
        title: 'Display name',
        trailing: const _RowChevron(),
        onTap: () {
          unawaited(HapticFeedback.selectionClick());
          unawaited(onEditDisplayName?.call(context));
        },
      ),
      AppListRow(
        key: const ValueKey('edit-profile-description'),
        title: 'Profile description',
        trailing: const _RowChevron(),
        onTap: () {
          unawaited(HapticFeedback.selectionClick());
          unawaited(onEditProfileDescription?.call(context));
        },
      ),
      AppListRow(
        key: const ValueKey('edit-profile-photo'),
        title: 'Edit photo',
        trailing: const _RowChevron(),
        onTap: () {
          unawaited(HapticFeedback.selectionClick());
          final scope = DuoToolbarScope.maybeOf(context);
          final strip = scope?.strip ?? FoldGeometry.of(context).strip;
          if (strip == null) {
            Navigator.of(
              context,
            ).push(immediatePageRoute<void>(builder: profileEditPageBuilder));
            return;
          }
          final windowSize =
              View.of(context).physicalSize / View.of(context).devicePixelRatio;
          final pane = ConversationPaneScope.maybeOf(context);
          if (windowSize.shortestSide < 600 && pane != null) {
            pane.open('settings-photo', profileEditPageBuilder, animate: false);
            return;
          }
          Widget editor(BuildContext routeContext) {
            final routeScope = DuoToolbarScope.maybeOf(routeContext);
            final routeStrip =
                routeScope?.strip ?? FoldGeometry.of(routeContext).strip;
            return DuoToolbarScope(
              strip: routeStrip,
              active: routeScope?.active ?? true,
              contentInsets:
                  routeScope?.contentInsets ??
                  EdgeInsets.only(
                    left: routeStrip?.side == FoldStripSide.left
                        ? routeStrip!.width
                        : 0,
                    right: routeStrip?.side == FoldStripSide.right
                        ? routeStrip!.width
                        : 0,
                  ),
              child: profileEditPageBuilder(routeContext),
            );
          }

          Navigator.of(context).push(
            windowSize.shortestSide < 600
                ? immediatePageRoute<void>(builder: editor)
                : BottomUpPageRoute<void>(
                    reducedMotion: MediaQuery.disableAnimationsOf(context),
                    builder: editor,
                  ),
          );
        },
      ),
    ],
  );
}
