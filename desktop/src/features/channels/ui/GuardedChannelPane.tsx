import type * as React from "react";

import { ChannelPane } from "./ChannelScreenLazyViews";

export function GuardedChannelPane(
  props: React.ComponentProps<typeof ChannelPane>,
) {
  return <ChannelPane {...props} />;
}
