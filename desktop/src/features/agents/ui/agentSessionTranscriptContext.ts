import * as React from "react";

export type AgentSessionTranscriptVariant = "default" | "compactPreview";

const AgentSessionTranscriptVariantContext =
  React.createContext<AgentSessionTranscriptVariant>("default");

export const AgentSessionTranscriptVariantProvider =
  AgentSessionTranscriptVariantContext.Provider;

export function useAgentSessionTranscriptVariant() {
  return React.useContext(AgentSessionTranscriptVariantContext);
}
