export const MAX_COMMUNITY_SECTIONS = 50;
export const MAX_SECTION_CHANNELS = 200;
export const MAX_SECTION_NAME_LEN = 80;

export type CommunitySection = {
  id: string;
  name: string;
  icon?: string;
  order: number;
  channelIds: string[];
};

export type CommunitySectionsPayload = {
  version: 1;
  sections: CommunitySection[];
};

export type CommunitySectionDraft = {
  name: string;
  icon?: string;
  channelIds: string[];
};

export type CommunitySectionSubscriptionStore = {
  version: 1;
  /** sectionId -> subscribed */
  subscribed: Record<string, boolean>;
};
