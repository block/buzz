import { useId } from "react";
import type { ComponentType } from "react";

import {
  Bot,
  BrainCircuit,
  Boxes,
  Cloud,
  Code2,
  Database,
  FileCode2,
  Globe2,
  HardDrive,
  Network,
  Server,
  Sparkles,
  Terminal,
  Webhook,
  Workflow,
} from "lucide-react";

import {
  SiGithub,
  SiNotion,
  SiClaude,
  SiFigma,
  SiLinear,
  SiDiscord,
  SiPostgresql,
  SiVercel,
} from "react-icons/si";

type FlappingBeeProps = {
  className?: string;
  index?: number;
};

type Integration = {
  name: string;

  icon: ComponentType<{
    className?: string;
    color?: string;
    size?: string | number;
  }>;

  color: string;

  background?: string;
};

/**
 * Unique Orbit integrations.
 *
 * IMPORTANT:
 * There are enough unique entries here to match the BEES array.
 * Therefore the landing screen doesn't need to repeat integrations.
 */
const INTEGRATIONS: Integration[] = [
  {
    name: "GitHub",
    icon: SiGithub,
    color: "#FFFFFF",
  },
  {
    name: "Claude",
    icon: SiClaude,
    color: "#D97757",
  },
  {
    name: "Linear",
    icon: SiLinear,
    color: "#FFFFFF",
  },
  {
    name: "Codex",
    icon: Terminal,
    color: "#10A37F",
  },
  {
    name: "Figma",
    icon: SiFigma,
    color: "#F24E1E",
  },
  {
    name: "Notion",
    icon: SiNotion,
    color: "#FFFFFF",
  },
  {
    name: "Discord",
    icon: SiDiscord,
    color: "#5865F2",
  },
  {
    name: "Vercel",
    icon: SiVercel,
    color: "#FFFFFF",
  },
  {
    name: "PostgreSQL",
    icon: SiPostgresql,
    color: "#4169E1",
  },
];

/**
 * Generic Orbit capabilities.
 *
 * These are here if you later increase the number of BEES beyond
 * the number of branded integrations.
 */
const ORBIT_CAPABILITIES: Integration[] = [
  {
    name: "MCP Server",
    icon: Server,
    color: "#A855F7",
  },

  {
    name: "AI Agent",
    icon: Bot,
    color: "#D60FD9",
  },

  {
    name: "AI Memory",
    icon: BrainCircuit,
    color: "#7C3AED",
  },

  {
    name: "Terminal Agent",
    icon: Terminal,
    color: "#22C55E",
  },

  {
    name: "Database",
    icon: Database,
    color: "#3B82F6",
  },

  {
    name: "Cloud",
    icon: Cloud,
    color: "#06B6D4",
  },

  {
    name: "Tools",
    icon: Boxes,
    color: "#8B5CF6",
  },

  {
    name: "Workflow",
    icon: Workflow,
    color: "#F59E0B",
  },

  {
    name: "Webhook",
    icon: Webhook,
    color: "#EC4899",
  },

  {
    name: "Network",
    icon: Network,
    color: "#06B6D4",
  },

  {
    name: "Code Agent",
    icon: Code2,
    color: "#8B5CF6",
  },

  {
    name: "File Context",
    icon: FileCode2,
    color: "#22C55E",
  },

  {
    name: "Knowledge Store",
    icon: HardDrive,
    color: "#3B82F6",
  },

  {
    name: "Web Context",
    icon: Globe2,
    color: "#06B6D4",
  },

  {
    name: "Agent Intelligence",
    icon: Sparkles,
    color: "#D60FD9",
  },
];

export function FlappingBee({
  className,
  index = 0,
}: FlappingBeeProps) {
  const maskId = `flapping-bee-cutouts-${useId().replace(
    /[^a-zA-Z0-9_-]/g,
    "",
  )}`;

  const wingLayer =
    "bee-wing-layer absolute left-0 top-0 h-full w-full";

  const wingSvg =
    "bee-wing block h-full w-full overflow-visible";

  /*
   * First consume every branded integration.
   *
   * Only after all branded integrations have been used do we
   * fall back to Orbit capability icons.
   *
   * This prevents:
   *
   * GitHub
   * Gmail
   * Notion
   * GitHub again
   * Gmail again
   *
   * etc.
   */
  const integration =
    INTEGRATIONS[index] ??
    ORBIT_CAPABILITIES[
      (index - INTEGRATIONS.length) %
        ORBIT_CAPABILITIES.length
    ];

  const Icon = integration.icon;

  return (
    <div
      aria-hidden="true"
      title={integration.name}
      className={[
        "buzz-mark",
        "bee-sprite",
        "group",
        "relative",
        "flex",
        "aspect-square",
        "items-center",
        "justify-center",
        className,
      ]
        .filter(Boolean)
        .join(" ")}
    >
      {/* =====================================================
          VERY SUBTLE AMBIENT GLOW

          Kept inside the icon bounds so it doesn't visually
          collide with neighbouring integrations.
      ===================================================== */}

      <div className={`${wingLayer} bee-wing-layer-left`}>
        <svg
          aria-hidden="true"
          className={`${wingSvg} bee-wing-left`}
          viewBox="0 0 100 100"
        >
          <defs>
            <radialGradient
              id={`${maskId}-glow`}
              cx="50%"
              cy="50%"
              r="50%"
            >
              <stop
                offset="0%"
                stopColor={integration.color}
                stopOpacity="0.22"
              />

              <stop
                offset="52%"
                stopColor={integration.color}
                stopOpacity="0.08"
              />

              <stop
                offset="100%"
                stopColor={integration.color}
                stopOpacity="0"
              />
            </radialGradient>
          </defs>

          <circle
            cx="50"
            cy="50"
            r="45"
            fill={`url(#${maskId}-glow)`}
          />
        </svg>
      </div>

      {/* =====================================================
          ICON CARD
      ===================================================== */}

      <div
        className="
          relative
          z-10
          flex
          h-[78%]
          w-[78%]
          items-center
          justify-center
          rounded-[25%]
          border
          border-white/[0.10]
          bg-[#111111]/90
          shadow-[0_5px_18px_rgba(0,0,0,0.24)]
          backdrop-blur-md
        "
      >
        <Icon
          className="h-[56%] w-[56%]"
          color={integration.color}
        />
      </div>
    </div>
  );
}