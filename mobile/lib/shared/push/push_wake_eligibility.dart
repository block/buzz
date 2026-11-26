import '../relay/nostr_models.dart';
import 'push_subscription.dart';

/// Whether a subscription filter matches [event]. This is a port of
/// `PushLeaseFilter.matches` in `BuzzPushKit`: change the two together.
bool pushFilterMatches(BuzzPushFilter filter, NostrEvent event) {
  if (!filter.kinds.contains(event.kind)) return false;
  final authors = filter.authors;
  if (authors != null && !authors.contains(event.pubkey.toLowerCase())) {
    return false;
  }
  final pTags = filter.pTags;
  if (pTags != null && !_hasAnyTag(event, 'p', pTags)) return false;
  final hTags = filter.hTags;
  if (hTags != null && !_hasAnyTag(event, 'h', hTags)) return false;
  final eTags = filter.eTags;
  if (eTags != null && !_hasAnyTag(event, 'e', eTags)) return false;
  return true;
}

/// Whether a subscription, with its ignore filters and hellthread
/// suppression, selects [event]. This is a port of
/// `PushLeaseMatcher.matches` in `BuzzPushKit`.
bool pushSubscriptionMatches(
  BuzzPushSubscription subscription,
  NostrEvent event,
) {
  if (!pushFilterMatches(subscription.filter, event)) return false;
  if (subscription.ignore.any((ignore) => pushFilterMatches(ignore, event))) {
    return false;
  }
  final maximum = subscription.suppress?.pTagsMax;
  if (maximum != null && _tagCount(event, 'p') > maximum) return false;
  return true;
}

/// A relay-signed workflow message names its owner in
/// `["p", owner, "", "owner"]` for attribution, not as a mention. Like the
/// relay, `p` matching and counting skip that tag on workflow messages.
bool _countsAsTag(List<String> tag, String name, {required bool workflow}) {
  if (tag.length < 2 || tag[0] != name) return false;
  return !(workflow && name == 'p' && tag.length >= 4 && tag[3] == 'owner');
}

bool _isWorkflow(NostrEvent event) => event.tags.any(
  (tag) => tag.length >= 2 && tag[0] == 'buzz:workflow' && tag[1] == 'true',
);

int _tagCount(NostrEvent event, String name) {
  final workflow = _isWorkflow(event);
  return event.tags
      .where((tag) => _countsAsTag(tag, name, workflow: workflow))
      .length;
}

bool _hasAnyTag(NostrEvent event, String name, List<String> values) {
  final expected = {for (final value in values) value.toLowerCase()};
  final workflow = _isWorkflow(event);
  return event.tags.any(
    (tag) =>
        _countsAsTag(tag, name, workflow: workflow) &&
        expected.contains(tag[1].toLowerCase()),
  );
}
