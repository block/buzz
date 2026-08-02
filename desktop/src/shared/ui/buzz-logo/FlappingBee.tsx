import { useId } from "react";
import type { ComponentType } from "react";

import {
  Bot,
  BrainCircuit,
  Boxes,
  Cloud,
  Code2,
  Database,
  Server,
  Sparkles,
  Terminal,
} from "lucide-react";

import {
  SiGithub,
  // SiSlack,
  SiGmail,
  SiNotion,
  SiClaude,
  SiFigma,
  SiLinear,
  SiJira,
  SiDiscord,
  SiGoogledrive,
  SiGooglechrome,
  SiPostgresql,
  // SiOpenai,
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
 * Keep the original component name so nothing else in the
 * Buzz/Orbit animation architecture needs to change.
 */
const INTEGRATIONS: Integration[] = [
  {
    name: "GitHub",
    icon: SiGithub,
    color: "#FFFFFF",
  },
  // {
  //   name: "Slack",
  //   icon: SiSlack,
  //   color: "#E01E5A",
  // },
  {
    name: "Gmail",
    icon: SiGmail,
    color: "#EA4335",
  },
  {
    name: "Notion",
    icon: SiNotion,
    color: "#FFFFFF",
  },
  {
    name: "Claude",
    icon: SiClaude,
    color: "#D97757",
  },
  // {
  //   name: "OpenAI / Codex",
  //   icon: SiOpenai,
  //   color: "#FFFFFF",
  // },
  {
    name: "Cursor",
    icon: Code2,
    color: "#FFFFFF",
  },
  {
    name: "Antigravity",
    icon: Sparkles,
    color: "#A855F7",
  },
  {
    name: "Google Drive",
    icon: SiGoogledrive,
    color: "#4285F4",
  },
  {
    name: "Discord",
    icon: SiDiscord,
    color: "#5865F2",
  },
  {
    name: "Figma",
    icon: SiFigma,
    color: "#F24E1E",
  },
  {
    name: "Linear",
    icon: SiLinear,
    color: "#FFFFFF",
  },
  {
    name: "Jira",
    icon: SiJira,
    color: "#2684FF",
  },
  {
    name: "Chrome",
    icon: SiGooglechrome,
    color: "#4285F4",
  },
  {
    name: "PostgreSQL",
    icon: SiPostgresql,
    color: "#4169E1",
  },
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

  const integration =
    INTEGRATIONS[index % INTEGRATIONS.length];

  const Icon = integration.icon;

  return (
    <div
      aria-hidden="true"
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
          COLORED AMBIENT GLOW
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
                stopOpacity="0.32"
              />

              <stop
                offset="45%"
                stopColor={integration.color}
                stopOpacity="0.12"
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
            r="48"
            fill={`url(#${maskId}-glow)`}
          />
        </svg>
      </div>

      {/* =====================================================
          ORBIT RING
      ===================================================== */}

      <div className={`${wingLayer} bee-wing-layer-right`}>
        <svg
          aria-hidden="true"
          className={`${wingSvg} bee-wing-right`}
          viewBox="0 0 100 100"
        >
          <circle
            cx="50"
            cy="50"
            r="32"
            fill="none"
            stroke={integration.color}
            strokeWidth="1"
            strokeOpacity="0.13"
          />

          <circle
            cx="50"
            cy="50"
            r="41"
            fill="none"
            stroke={integration.color}
            strokeWidth="0.6"
            strokeOpacity="0.06"
          />
        </svg>
      </div>

      {/* =====================================================
          INTEGRATION ICON
      ===================================================== */}

      <div
        className="
          relative
          z-10
          flex
          h-[74%]
          w-[74%]
          items-center
          justify-center
          rounded-[26%]
          border
          border-white/[0.12]
          bg-[#111111]/80
          shadow-[0_5px_20px_rgba(0,0,0,0.28)]
          backdrop-blur-md
          transition-transform
          duration-300
          group-hover:scale-110
        "
      >
        <Icon
          className="h-[55%] w-[55%]"
          color={integration.color}
        />
      </div>

      {/* =====================================================
          SMALL ORBIT BRAND SPARK
      ===================================================== */}

      <svg
        aria-hidden="true"
        className="
          absolute
          -right-[5%]
          -top-[6%]
          z-20
          h-[29%]
          w-[29%]
          overflow-visible
        "
        viewBox="0 0 100 100"
      >
        <defs>
          <linearGradient
            id={`${maskId}-orbit-gradient`}
            x1="20"
            y1="90"
            x2="78"
            y2="5"
            gradientUnits="userSpaceOnUse"
          >
            <stop
              offset="0%"
              stopColor="#3D42D9"
            />

            <stop
              offset="48%"
              stopColor="#7626E2"
            />

            <stop
              offset="100%"
              stopColor="#D60FD9"
            />
          </linearGradient>
        </defs>

        <path
          d="
            M50 3

            C53 25
             59 38
             70 43

            C76 46
             84 48
             97 50

            C84 53
             76 56
             70 61

            C59 70
             54 82
             50 97

            C47 82
             41 70
             30 61

            C24 56
             16 53
             3 50

            C16 47
             24 45
             30 41

            C41 34
             47 22
             50 3

            Z
          "
          fill={`url(#${maskId}-orbit-gradient)`}
        />
      </svg>
    </div>
  );
}