part of '../thread_detail_page.dart';

/// The pinned "up one level" row and the header height it adds. [row] is
/// null, and [height] zero, while the thread's conversation is unresolved.
typedef _ThreadUpLevel = ({Widget? row, double height});

/// The head each open thread page shows, by its route, so a sub-thread can
/// tell when its parent thread is the page directly beneath it.
final _openThreadHeads = Expando<String>();

final _markdownLink = RegExp(r'\[([^\]]*)\]\([^)]*\)');
final _markdownMarks = RegExp(r'[*_`~>#]');
final _whitespaceRun = RegExp(r'\s+');

/// A thread named by its head's text, as desktop titles a thread tab.
String _threadSnippet(String content) => content
    .replaceAllMapped(_markdownLink, (match) => match[1]!)
    .replaceAll(_markdownMarks, '')
    .replaceAll(_whitespaceRun, ' ')
    .trim()
    .characters
    .take(80)
    .toString();

/// The conversation named as desktop's thread panel and Inbox rows name it.
String _threadConversationLabel(Channel channel, String displayName) =>
    channel.isDm
    ? 'DM · $displayName'
    : channel.isSolo
    ? displayName
    : '#$displayName';

/// Resolves where "up" leads from [page]: the thread whose head is the
/// direct parent of this page's head, or the conversation for a top-level
/// thread. A parent message that is not loaded is fetched once the thread's
/// own replies have settled without it, and the row waits, inert, until then.
_ThreadUpLevel _useThreadUpLevel(
  BuildContext context,
  WidgetRef ref,
  ThreadDetailPage page,
  List<TimelineMessage> messages,
  String? currentPubkey, {
  required bool repliesSettled,
}) {
  final head = page.threadHead;
  final channelId = page.channelId;
  final channel = ref.watch(channelByIdProvider(channelId));
  final conversation = channel == null
      ? null
      : _threadConversationLabel(
          channel,
          _threadConversationName(ref, channel, currentPubkey),
        );
  final route = ModalRoute.of(context);
  useEffect(() {
    if (route != null) _openThreadHeads[route] = head.id;
    return null;
  }, [route, head.id]);

  final parentId = head.parentId;
  final loadedParent = parentId == null
      ? null
      : messages.where((message) => message.id == parentId).firstOrNull;
  // The lookup below stops pinning the parent once it arrives.
  final knownParent = useRef<TimelineMessage?>(null);
  if (loadedParent != null) knownParent.value = loadedParent;
  final parent = parentId == null ? null : loadedParent ?? knownParent.value;
  final lookupSettled = useState(false);
  final needsLookup = parentId != null && parent == null && repliesSettled;
  useEffect(() {
    if (!needsLookup) return null;
    final notifier = ref.read(channelMessagesProvider(channelId).notifier);
    var cancelled = false;
    Future<void> lookUp() async {
      try {
        await notifier.loadEventsById({parentId});
      } catch (error) {
        debugPrint('thread: failed to load parent message: $error');
      }
      // A parent that cannot be loaded leaves the conversation as the way up.
      if (!cancelled) lookupSettled.value = true;
    }

    unawaited(lookUp());
    return () {
      cancelled = true;
      notifier.releaseDeepLinkEvents({parentId});
    };
  }, [needsLookup, parentId, channelId]);

  if (channel == null || conversation == null) return (row: null, height: 0);

  void open(Route<void> Function() target, bool Function(Route<dynamic>) isIt) {
    // A second tap lands after this page has already left the top.
    if (route == null || !route.isCurrent) return;
    final navigator = Navigator.of(context);
    final below = appRouteObserver.routeBelow(route);
    if (below != null && isIt(below)) {
      unawaited(navigator.maybePop());
    } else {
      unawaited(navigator.push(target()));
    }
  }

  void openConversation() {
    final rootId = head.rootId ?? head.id;
    final routes = ref.read(openChannelRoutesProvider);
    open(
      () => MaterialPageRoute<void>(
        builder: (_) =>
            ChannelDetailPage(channel: channel, initialMessageId: rootId),
      ),
      (below) {
        if (!routes.shows(channelId, below)) return false;
        // An Inbox or push landing left the channel at its newest message.
        routes.reveal(below, rootId);
        return true;
      },
    );
  }

  void openParentThread(TimelineMessage parent) => open(
    () => MaterialPageRoute<void>(
      builder: (_) => ThreadDetailPage(
        threadHead: parent,
        allMessages: messages,
        channelId: channelId,
        currentPubkey: currentPubkey,
        isMember: page.isMember,
        isArchived: page.isArchived,
        initialMessageId: head.id,
      ),
    ),
    (below) => _openThreadHeads[below] == parent.id,
  );

  final Widget row;
  if (parent != null) {
    final snippet = _threadSnippet(parent.content);
    row = _ThreadUpLevelRow(
      target: snippet.isEmpty ? 'thread' : 'thread · $snippet',
      semanticLabel: 'Go to parent thread',
      onPressed: () => openParentThread(parent),
    );
  } else if (parentId != null && !lookupSettled.value) {
    row = const _ThreadUpLevelRow(
      target: 'thread',
      semanticLabel: 'Loading parent thread',
      onPressed: null,
    );
  } else {
    row = _ThreadUpLevelRow(
      target: conversation,
      semanticLabel: 'Go to $conversation',
      onPressed: openConversation,
    );
  }
  return (row: row, height: _threadUpLevelRowHeight(context));
}
