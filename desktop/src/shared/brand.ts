/** Public-facing identity for this distribution. */
export const PRODUCT_NAME = "Zorro" as const;
export const PRODUCT_TERM_NAME = `${PRODUCT_NAME} Term` as const;

/** Theme IDs are public appearance choices; legacy `buzz-*` storage keys remain stable. */
export const PRODUCT_THEME = {
  light: "zorro",
  dark: "zorro-dark",
} as const;

/** Built-in starter team shown during onboarding and in quick-agent pickers. */
export const STARTER_AGENT_BRAND = [
  {
    name: "Diego",
    personaId: "builtin:diego",
    animationUrl: "/onboarding/starter-team/diego.png",
    role: "lead",
  },
  {
    name: "Murietta",
    personaId: "builtin:murietta",
    animationUrl: "/onboarding/starter-team/murietta.png",
    role: "teammate",
  },
  {
    name: "Montero",
    personaId: "builtin:montero",
    animationUrl: "/onboarding/starter-team/montero.png",
    role: "teammate",
  },
] as const;
