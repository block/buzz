import { useId } from "react";

/**
 * Orbit AI static brand mark.
 *
 * Transparent background.
 * Component and variable names are intentionally kept the same
 * so existing imports/usages do not need to change.
 */
export function BuzzMark({ className }: { className?: string }) {
  const maskId = `buzz-mark-cutouts-${useId().replace(
    /[^a-zA-Z0-9_-]/g,
    "",
  )}`;

  return (
    <svg
      aria-hidden="true"
      className={["buzz-mark", className].filter(Boolean).join(" ")}
      viewBox="0 0 1024 1024"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      style={{
        background: "transparent",
      }}
    >
      <defs>
        {/* =====================================================
            ORBIT SILVER RING GRADIENT
        ===================================================== */}

        <linearGradient
          id={`${maskId}-ring-gradient`}
          x1="418.986"
          y1="170.667"
          x2="-85.7957"
          y2="748.696"
          gradientUnits="userSpaceOnUse"
        >
          <stop
            offset="4.32692%"
            stopColor="#FFFFFF"
          />

          <stop
            offset="80.7692%"
            stopColor="#B9B9BE"
          />
        </linearGradient>

        {/* =====================================================
            ORBIT PURPLE → BLUE GRADIENT
        ===================================================== */}

        <linearGradient
          id={`${maskId}-spark-gradient`}
          x1="709.762"
          y1="0"
          x2="709.762"
          y2="628.053"
          gradientUnits="userSpaceOnUse"
        >
          <stop
            offset="0%"
            stopColor="#F109ED"
          />

          <stop
            offset="83.1731%"
            stopColor="#3D42D9"
          />
        </linearGradient>
      </defs>

      {/* =====================================================
          ORBIT RING
      ===================================================== */}

      <path
        d="
          M743.02 476.311

          C770.07 551.99
          770.953 634.859
          745.517 711.129

          C720.086 787.382
          669.964 852.225
          603.746 895.156

          C537.556 938.068
          459.142 956.607
          381.422 947.903

          C303.693 939.199
          230.976 903.726
          175.373 847.036

          C119.745 790.321
          84.5999 715.763
          76.0387 635.609

          C67.4769 555.449
          86.0538 474.812
          128.518 407.068

          C170.967 339.348
          234.651 288.684
          308.822 263.132

          C382.971 237.588
          463.415 238.57
          536.953 265.928
        "
        stroke={`url(#${maskId}-ring-gradient)`}
        strokeWidth="148"
        strokeLinecap="butt"
        fill="none"
      />

      {/* =====================================================
          ORBIT SPARK
      ===================================================== */}

      <path
        d="
          M744.119 390.827

          C712.276 401.067
          556.183 479.875
          395.523 628.053

          C545.561 457.83
          612.295 373.558
          631.83 273.067

          C646.39 177.87
          646.723 114.299
          631.83 0

          C690.106 158.381
          712.276 186.027
          754.174 179.2

          C787.693 167.253
          839.796 156.256
          1024 0

          C862.302 183.317
          832.943 247.467
          836.295 264.533

          C839.647 281.6
          869.193 342.844
          1012.27 390.827

          C903.263 367.283
          775.961 380.587
          744.119 390.827

          Z
        "
        fill={`url(#${maskId}-spark-gradient)`}
      />
    </svg>
  );
}