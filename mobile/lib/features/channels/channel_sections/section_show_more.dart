import 'package:flutter/material.dart';
import 'package:duo_kit/duo_kit.dart';
import '../../../shared/widgets/duo_toolbar.dart';
import '../../../shared/widgets/adaptive_conversation_pane.dart';
import '../../../shared/widgets/frosted_app_bar.dart';
import 'package:flutter/foundation.dart';
import '../../../shared/widgets/concentric_sheet_surface.dart';
import '../../../shared/widgets/ios_glass_navigation_button.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../../shared/identity_names/identity_names_provider.dart';
import '../../../shared/relay/relay_provider.dart';
import '../../../shared/theme/buzz_icons.dart';
import '../../../shared/theme/theme.dart';
import '../channel.dart';
import '../channels_page/filters.dart';
import '../dm_channel_labels.dart';

typedef SectionChatRowBuilder =
    Widget Function(Channel channel, [VoidCallback? onTap]);

/// The inventory owns membership, ordering and row state. Keep its open route
/// subscribed even when the preview shrinks enough to remove the See all row.
typedef SectionChatContent = ({
  List<Channel> channels,
  SectionChatRowBuilder buildRow,
  Widget Function(Channel) buildDetail,
  String title,
});

class SectionShowMore extends StatelessWidget {
  const SectionShowMore({
    super.key,
    this.liveContent,
    this.leadingInset,
    this.horizontalInset = Grid.gutter,
    required this.title,
    required this.channels,
    required this.buildDetail,
    required this.buildRow,
    this.rowInset = 0,
  });
  final SectionChatContent Function(WidgetRef)? liveContent;
  final double? leadingInset;
  final double horizontalInset;
  final double rowInset;
  final SectionChatRowBuilder buildRow;
  final String title;
  final List<Channel> channels;
  final Widget Function(Channel) buildDetail;

  @override
  Widget build(BuildContext context) => Semantics(
    button: true,
    label: 'See all $title',
    child: InkWell(
      onTap: () async {
        final options = context
            .getInheritedWidgetOfExactType<MessagesSectionOptions>();
        await Navigator.of(context, rootNavigator: true).push<void>(
          _SectionChatsRoute(
            builder: (routeContext) {
              Widget pageFor(SectionChatContent content) => _SectionChatsPage(
                title: content.title,
                channels: content.channels,
                buildRow: content.buildRow,
                rowInset: rowInset,
                buildDetail: content.buildDetail,
              );
              final live = liveContent;
              final page = live == null
                  ? pageFor((
                      title: title,
                      channels: channels,
                      buildRow: buildRow,
                      buildDetail: buildDetail,
                    ))
                  : Consumer(builder: (_, ref, _) => pageFor(live(ref)));
              final list = options == null
                  ? page
                  : MessagesSectionOptions(child: page);
              return AdaptiveConversationDestination(
                requireFoldForSplit: true,
                preferredListWidth: 375,
                listToolbarSlots: 1,
                child: list,
              );
            },
          ),
        );
      },
      child: Padding(
        padding: EdgeInsets.only(
          left: leadingInset ?? horizontalInset,
          right: horizontalInset,
          top: Grid.inventoryRowInset,
          bottom: Grid.inventoryRowInset,
        ),
        child: Row(
          children: [
            Icon(
              BuzzIcons.arrowRight,
              size: Grid.inventoryIconSize,
              color: context.colors.onSurfaceVariant,
            ),
            const SizedBox(width: Grid.inventoryLabelGap),
            ExcludeSemantics(
              child: Text(
                'See all',
                style: navigationChatTitleTextStyle.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ),
          ],
        ),
      ),
    ),
  );
}

class _SectionChatsRoute extends MaterialPageRoute<void> {
  _SectionChatsRoute({required super.builder});

  @override
  bool canTransitionTo(TransitionRoute<dynamic> nextRoute) => false;
}

class _SectionChatsPage extends HookConsumerWidget {
  const _SectionChatsPage({
    required this.title,
    required this.channels,
    required this.buildRow,
    required this.rowInset,
    required this.buildDetail,
  });
  final SectionChatRowBuilder buildRow;
  final double rowInset;
  final Widget Function(Channel) buildDetail;
  final String title;
  final List<Channel> channels;
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final editing = useState(false);
    final focus = useFocusNode();
    useEffect(() {
      void activate() {
        if (focus.hasFocus) editing.value = true;
      }

      focus.addListener(activate);
      return () => focus.removeListener(activate);
    }, [focus]);
    final controller = useTextEditingController();
    final query = useListenableSelector(
      controller,
      () => controller.text.trim().toLowerCase(),
    );
    final names = ref.watch(identityNameSourcesProvider);
    final viewer = ref.watch(myPubkeyProvider);
    final matches = channels
        .where(
          (c) => resolveDmChannelDisplayLabel(
            c,
            names: names,
            currentPubkey: viewer,
          ).toLowerCase().contains(query),
        )
        .toList();
    final ios = defaultTargetPlatform == TargetPlatform.iOS;
    final toolbar = DuoToolbarScope.maybeOf(context);
    final strip = toolbar == null
        ? FoldGeometry.of(context).strip
        : toolbar.strip;
    void cancel() {
      focus.unfocus();
      controller.clear();
      editing.value = false;
    }

    final duration = MediaQuery.disableAnimationsOf(context)
        ? Duration.zero
        : const Duration(milliseconds: 240);
    final transition = useAnimationController(duration: duration);
    useEffect(() {
      transition.animateTo(
        editing.value ? 1 : 0,
        duration: duration,
        curve: Curves.easeInOutCubic,
      );
      return null;
    }, [editing.value, duration]);
    final progress = useAnimation(transition);
    final searchColor = context.colors.brightness == Brightness.light
        ? lightColorScheme.surfaceContainerLow
        : darkColorScheme.surfaceContainerHigh;
    final splitDuo =
        ios &&
        strip != null &&
        usesConversationPanes(
          context,
          strip: strip,
          windowSize:
              View.of(context).physicalSize / View.of(context).devicePixelRatio,
        );
    final regularPhone = ios && strip == null && isCompactDuoWindow(context);
    final idleInset = regularPhone ? Grid.xs : Grid.xxs;
    final activeInset = splitDuo ? Grid.xxs + Grid.xs : Grid.xxs;
    final searchTopInset = idleInset + (activeInset - idleInset) * progress;
    final safeTop = MediaQuery.paddingOf(context).top;
    final fullHeight = frostedAppBarHeight(context);
    final detailOpen =
        ConversationPaneScope.maybeOf(context)?.selectedId != null;
    return PopScope<void>(
      canPop: detailOpen || !editing.value,
      onPopInvokedWithResult: (popped, _) {
        if (!popped && !detailOpen) cancel();
      },
      child: DuoToolbarPortal(
        strip: ios ? strip : null,
        active:
            (toolbar?.active ?? true) &&
            (ModalRoute.isCurrentOf(context) ?? true),
        actions: [
          IosNavigationAction(
            label: 'Close',
            symbol: 'xmark',
            onPressed: () => Navigator.of(context, rootNavigator: true).pop(),
          ),
        ],
        child: Scaffold(
          body: Stack(
            children: [
              Padding(
                padding: toolbar?.contentInsets ?? EdgeInsets.zero,
                child: SafeArea(
                  top: !ios,
                  child: Padding(
                    padding: EdgeInsets.only(
                      top: ios
                          ? safeTop + (fullHeight - safeTop) * (1 - progress)
                          : 0,
                    ),
                    child: Column(
                      children: [
                        if (!ios)
                          AnimatedSize(
                            alignment: Alignment.topCenter,
                            duration: MediaQuery.disableAnimationsOf(context)
                                ? Duration.zero
                                : const Duration(milliseconds: 200),
                            curve: Curves.easeOutCubic,
                            child: editing.value
                                ? const SizedBox.shrink()
                                : SizedBox(
                                    height: kToolbarHeight,
                                    child: AppBar(
                                      primary: false,
                                      leading: Center(
                                        child: IconButton(
                                          tooltip: MaterialLocalizations.of(
                                            context,
                                          ).backButtonTooltip,
                                          style: androidNavigationButtonStyle(
                                            context,
                                          ),
                                          icon: const BackButtonIcon(),
                                          onPressed: () =>
                                              Navigator.maybePop(context),
                                        ),
                                      ),
                                      title: Text(title),
                                    ),
                                  ),
                          ),
                        Padding(
                          padding: EdgeInsets.fromLTRB(
                            Grid.xs,
                            searchTopInset,
                            Grid.xs,
                            Grid.xxs,
                          ),
                          child: Row(
                            children: [
                              Expanded(
                                child: DecoratedBox(
                                  decoration: BoxDecoration(
                                    color: Color.lerp(
                                      searchColor,
                                      searchColor.withValues(alpha: 0),
                                      ios ? progress : 0,
                                    ),
                                    borderRadius: BorderRadius.circular(
                                      Radii.full,
                                    ),
                                  ),
                                  child: Stack(
                                    children: [
                                      Positioned.fill(
                                        child: ConcentricSheetSurface(
                                          enabled: ios && editing.value,
                                          usesGlass: ios && editing.value,
                                          capsule: true,
                                          minimumRadius: Radii.full,
                                          providesSheetSurface: false,
                                          color: searchColor,
                                          padding: EdgeInsets.zero,
                                          child: const SizedBox.expand(),
                                        ),
                                      ),
                                      TextField(
                                        controller: controller,
                                        focusNode: focus,
                                        textInputAction: TextInputAction.search,
                                        textAlignVertical:
                                            TextAlignVertical.center,
                                        style: searchInputTextStyle,
                                        decoration: const InputDecoration(
                                          hintText: 'Search...',
                                          isDense: true,
                                          contentPadding: EdgeInsets.symmetric(
                                            vertical: Grid.twelve,
                                          ),
                                          prefixIconConstraints:
                                              BoxConstraints.tightFor(
                                                width: 44,
                                                height: 44,
                                              ),
                                          prefixIcon: Icon(
                                            BuzzIcons.search,
                                            size: 22,
                                          ),
                                          filled: false,
                                          border: InputBorder.none,
                                          enabledBorder: InputBorder.none,
                                          focusedBorder: InputBorder.none,
                                        ),
                                      ),
                                    ],
                                  ),
                                ),
                              ),
                              if (progress > 0)
                                ClipRect(
                                  child: Align(
                                    widthFactor: progress,
                                    child: Opacity(
                                      opacity: progress,
                                      child: IgnorePointer(
                                        ignoring: !editing.value,
                                        child: Row(
                                          mainAxisSize: MainAxisSize.min,
                                          children: [
                                            const SizedBox(width: Grid.xxs),
                                            if (ios)
                                              IosGlassNavigationButton(
                                                icon: IosGlassNavigationIcon
                                                    .close,
                                                semanticLabel: 'Cancel search',
                                                onPressed: cancel,
                                              )
                                            else
                                              IconButton(
                                                tooltip: 'Cancel search',
                                                icon: const Icon(Icons.close),
                                                onPressed: cancel,
                                              ),
                                          ],
                                        ),
                                      ),
                                    ),
                                  ),
                                ),
                            ],
                          ),
                        ),
                        Expanded(
                          child: matches.isEmpty
                              ? const Center(child: Text('No results'))
                              : CustomScrollView(
                                  key: PageStorageKey(
                                    editing.value
                                        ? 'section-search'
                                        : 'section-browse',
                                  ),
                                  keyboardDismissBehavior:
                                      ScrollViewKeyboardDismissBehavior.onDrag,
                                  slivers: [
                                    SliverPadding(
                                      padding: EdgeInsets.symmetric(
                                        horizontal: rowInset,
                                      ),
                                      sliver: SliverList.builder(
                                        itemCount: matches.length,
                                        itemBuilder: (context, i) {
                                          final channel = matches[i];
                                          return buildRow(
                                            channel,
                                            () => openConversationPane(
                                              context,
                                              channel.id,
                                              (_) => buildDetail(channel),
                                            ),
                                          );
                                        },
                                      ),
                                    ),
                                  ],
                                ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
              if (ios && progress < 1)
                Positioned.fill(
                  child: IgnorePointer(
                    ignoring: editing.value,
                    child: Opacity(
                      opacity: 1 - progress,
                      child: Transform.translate(
                        offset: Offset(0, -(fullHeight - safeTop) * progress),
                        child: Stack(
                          children: [
                            FrostedAppBar(
                              nativeTitle: title,
                              leadingInlineTitleOnDuo: true,
                              nativePaneSoftEdge: true,
                              automaticallyImplyLeading: false,
                              alwaysFrosted: true,
                              nativeActions: const [],
                              nativeLeading: strip != null
                                  ? null
                                  : IosNavigationAction(
                                      label: 'Back',
                                      symbol: 'chevron.backward',
                                      onPressed: () => Navigator.pop(context),
                                    ),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}
