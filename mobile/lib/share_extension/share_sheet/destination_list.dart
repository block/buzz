part of '../share_sheet.dart';

/// The search field, then New Solo chat and the Bestie DM, then every
/// conversation newest first, filtered by the search.
class _DestinationList extends StatelessWidget {
  const _DestinationList({
    required this.loaded,
    required this.search,
    required this.selected,
    required this.enabled,
    required this.onSelect,
    required this.bestieName,
  });

  final AsyncSnapshot<ShareDestinations> loaded;
  final TextEditingController search;
  final String? selected;
  final bool enabled;
  final ValueChanged<String> onSelect;

  /// Who a new Solo chat starts with; null until the app has published the
  /// Bestie.
  final String? bestieName;

  @override
  Widget build(BuildContext context) {
    final query = search.text.trim().toLowerCase();
    final data = loaded.data;
    final bestie = data?.bestie;
    final recent = [
      for (final c in data?.recent ?? const <ShareConversation>[])
        if (query.isEmpty || c.name.toLowerCase().contains(query)) c,
    ];
    Widget row(ShareConversation c, {String? subtitle}) => ListTile(
      enabled: enabled,
      selected: c.id == selected,
      leading: Icon(
        c.isSolo
            ? BuzzIcons.messageCircle
            : c.isDm
            ? Icons.person_outline
            : Icons.tag,
      ),
      title: Text(c.name, maxLines: 1, overflow: TextOverflow.ellipsis),
      subtitle: subtitle == null ? null : Text(subtitle),
      trailing: c.id == selected ? const Icon(Icons.check) : null,
      onTap: () => onSelect(c.id),
    );
    return ListView(
      keyboardDismissBehavior: ScrollViewKeyboardDismissBehavior.onDrag,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(Grid.xs, Grid.xxs, Grid.xs, 0),
          child: TextField(
            controller: search,
            enabled: enabled,
            decoration: const InputDecoration(
              prefixIcon: Icon(Icons.search),
              hintText: 'Search conversations',
              isDense: true,
            ),
          ),
        ),
        ListTile(
          enabled: enabled && bestieName != null,
          selected: selected == newSoloChatId,
          leading: const Icon(Icons.add_comment_outlined),
          title: const Text('New Solo chat'),
          subtitle: Text(
            bestieName == null
                ? 'Open Buzz once to choose who it starts with'
                : 'With ${bestieName!.isEmpty ? 'your Bestie' : bestieName}',
          ),
          trailing: selected == newSoloChatId ? const Icon(Icons.check) : null,
          onTap: () => onSelect(newSoloChatId),
        ),
        if (bestie != null && query.isEmpty) row(bestie, subtitle: 'Bestie'),
        if (loaded.hasError)
          ListTile(title: Text('Couldn’t load conversations: ${loaded.error}'))
        else if (data == null)
          const Padding(
            padding: EdgeInsets.all(Grid.sm),
            child: Center(child: CircularProgressIndicator()),
          ),
        for (final c in recent) row(c),
      ],
    );
  }
}
