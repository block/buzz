import type {
  ChannelSection,
  ChannelSectionStore,
  SmartSectionRule,
} from "./channelSectionsStorage";
import { MAX_SMART_SECTION_PATTERN_LENGTH } from "./channelSectionsStorage";

export type SmartSectionChannel = {
  id: string;
  name: string;
};

export function getSmartSectionRuleError(
  rule: SmartSectionRule,
): string | null {
  if (rule.pattern.trim().length === 0) return "Enter a channel name pattern.";
  if (rule.pattern.length > MAX_SMART_SECTION_PATTERN_LENGTH) {
    return `Keep the pattern under ${MAX_SMART_SECTION_PATTERN_LENGTH} characters.`;
  }
  if (!rule.isRegex) return null;
  try {
    new RegExp(rule.pattern, rule.caseSensitive ? "" : "i");
    return null;
  } catch {
    return "Enter a valid regular expression.";
  }
}

export function channelMatchesSmartRule(
  channelName: string,
  rule: SmartSectionRule,
): boolean {
  if (getSmartSectionRuleError(rule)) return false;
  if (rule.isRegex) {
    return new RegExp(rule.pattern, rule.caseSensitive ? "" : "i").test(
      channelName,
    );
  }
  const pattern = rule.caseSensitive
    ? rule.pattern
    : rule.pattern.toLocaleLowerCase();
  const candidate = rule.caseSensitive
    ? channelName
    : channelName.toLocaleLowerCase();
  return candidate.includes(pattern);
}

export function matchingChannelsForSection<T extends SmartSectionChannel>(
  section: ChannelSection,
  channels: readonly T[],
): T[] {
  const rule = section.smartRule;
  if (!rule) return [];
  return channels.filter((channel) =>
    channelMatchesSmartRule(channel.name, rule),
  );
}

/**
 * Assigns only channels without a saved placement. Persisting the assignment
 * makes the rule a one-time arrival decision: later manual moves remain stable.
 */
export function applyAutomaticSmartSectionAssignments(
  store: ChannelSectionStore,
  channels: readonly SmartSectionChannel[],
): ChannelSectionStore {
  const smartSections = store.sections
    .filter((section) => section.smartRule)
    .slice()
    .sort((left, right) => left.order - right.order);
  if (smartSections.length === 0) return store;

  const manuallyUnassigned = new Set(store.manuallyUnassignedChannelIds ?? []);
  let assignments: Record<string, string> | null = null;
  for (const channel of channels) {
    if (store.assignments[channel.id] || manuallyUnassigned.has(channel.id)) {
      continue;
    }
    const matchingSection = smartSections.find(
      (section) =>
        section.smartRule &&
        channelMatchesSmartRule(channel.name, section.smartRule),
    );
    if (!matchingSection) continue;
    assignments ??= { ...store.assignments };
    assignments[channel.id] = matchingSection.id;
  }
  return assignments ? { ...store, assignments } : store;
}

export function assignChannelsToSection(
  store: ChannelSectionStore,
  channelIds: readonly string[],
  sectionId: string,
): ChannelSectionStore {
  if (channelIds.length === 0) return store;
  const movedIds = new Set(channelIds);
  const assignments = { ...store.assignments };
  for (const channelId of channelIds) {
    delete assignments[channelId];
    assignments[channelId] = sectionId;
  }
  const manuallyUnassignedChannelIds = (
    store.manuallyUnassignedChannelIds ?? []
  ).filter((channelId) => !movedIds.has(channelId));
  return {
    ...store,
    assignments,
    ...(manuallyUnassignedChannelIds.length > 0
      ? { manuallyUnassignedChannelIds }
      : { manuallyUnassignedChannelIds: undefined }),
  };
}

export function manuallyUnassignChannel(
  store: ChannelSectionStore,
  channelId: string,
): ChannelSectionStore {
  const assignments = { ...store.assignments };
  delete assignments[channelId];
  return {
    ...store,
    assignments,
    manuallyUnassignedChannelIds: [
      ...(store.manuallyUnassignedChannelIds ?? []).filter(
        (existingId) => existingId !== channelId,
      ),
      channelId,
    ],
  };
}
