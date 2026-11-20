part of '../channels_page.dart';

class _ChannelHeaderActions extends StatelessWidget {
  const _ChannelHeaderActions({required this.onSearch, required this.onCreate});
  final VoidCallback? onSearch;
  final VoidCallback onCreate;

  @override
  Widget build(BuildContext context) => Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      if (onSearch != null) ...[
        _surface(
          context,
          IconButton(
            tooltip: 'Search',
            icon: const Icon(BuzzIcons.search300),
            onPressed: onSearch,
          ),
        ),
        const SizedBox(width: Grid.xs),
      ],
      _surface(
        context,
        IconButton(
          tooltip: 'Create',
          icon: const Icon(BuzzIcons.plus),
          onPressed: onCreate,
        ),
      ),
    ],
  );
  Widget _surface(BuildContext context, Widget child) => ClipRRect(
    borderRadius: BorderRadius.circular(Radii.full),
    child: ColoredBox(color: context.controlSurface, child: child),
  );
}
