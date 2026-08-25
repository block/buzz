import { registerBuiltinChannelFeatures } from "./builtins";

export {
  channelGlyph,
  classifyChannel,
  classifyChannelWith,
  getChannelPlugins,
  registerChannelFeature,
} from "./registry";
export type {
  ChannelBinding,
  ChannelClassifyInput,
  ChannelFeaturePlugin,
} from "./types";

// Registering here (module scope, run once per module load thanks to ESM
// caching) means any call site that imports from this barrel gets the
// built-in dm/private-channel/forum/stream plugins for free, mirroring how
// `shared/features/manifest` loads its manifest at import time rather than
// requiring an explicit bootstrap call from `App.tsx`.
registerBuiltinChannelFeatures();
