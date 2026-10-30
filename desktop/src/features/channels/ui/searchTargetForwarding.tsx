import * as React from "react";

import type { ForumChannelContent } from "./ForumChannelContent";
import type { GuardedChannelPane } from "./GuardedChannelPane";
import type { ChannelScreenProps } from "./ChannelScreen.types";

type SearchTarget = Pick<
  ChannelScreenProps,
  "targetSearchMessageId" | "targetSearchQuery"
>;

export const renderSearchAwareForum = (
  node: React.ReactElement<React.ComponentProps<typeof ForumChannelContent>>,
  target: SearchTarget,
) => React.cloneElement(node, target);

export const renderSearchAwareChannel = (
  node: React.ReactElement<React.ComponentProps<typeof GuardedChannelPane>>,
  target: SearchTarget,
) => React.cloneElement(node, target);
