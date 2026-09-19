import type * as React from "react";

import {
  type PlaygroundBezelEdge,
  type PlaygroundBezelNub,
  type PlaygroundDeviceBezel,
  playgroundDeviceBezel,
  playgroundDeviceBezelOuterSize,
  playgroundDeviceNubGutter,
  scalePlaygroundDeviceBezel,
} from "../lib/deviceBezel";
import {
  PLAYGROUND_DEVICE_SCALE_DEFAULT,
  playgroundDeviceScaleFactor,
  playgroundDeviceViewport,
  scalePlaygroundDeviceViewport,
  type PlaygroundDevice,
} from "../lib/devices";

/** Hardware chrome around the mobile-stage screen hole. */
export function DeviceBezel({
  children,
  device,
  orientation,
  scalePercent = PLAYGROUND_DEVICE_SCALE_DEFAULT,
}: {
  children: React.ReactNode;
  device: PlaygroundDevice;
  orientation: "portrait" | "landscape";
  /** Museum preview scale in 25% steps (50–200). Default 100. */
  scalePercent?: number;
}) {
  const factor = playgroundDeviceScaleFactor(scalePercent);
  const bezel = scalePlaygroundDeviceBezel(
    playgroundDeviceBezel(device, orientation),
    factor,
  );
  const viewport = scalePlaygroundDeviceViewport(
    playgroundDeviceViewport(device, orientation),
    scalePercent,
  );
  const outer = playgroundDeviceBezelOuterSize(viewport, bezel);
  const gutter = playgroundDeviceNubGutter(bezel);

  return (
    <div className="relative" style={{ padding: gutter }}>
      <div
        className="relative"
        style={{
          width: outer.width,
          height: outer.height,
          borderRadius: bezel.outerRadius,
          boxShadow:
            "0 18px 48px rgb(0 0 0 / 0.28), 0 0 0 1px rgb(255 255 255 / 0.06)",
        }}
      >
        <HardwareNubs bezel={bezel} />
        <div
          className="relative overflow-hidden"
          data-bezel="hardware"
          data-chrome={bezel.chrome}
          data-family={bezel.family}
          data-orientation={orientation}
          data-scale={String(Math.round(factor * 100))}
          data-testid="playground-device-frame"
          style={{
            width: outer.width,
            height: outer.height,
            paddingTop: bezel.padding.top,
            paddingRight: bezel.padding.right,
            paddingBottom: bezel.padding.bottom,
            paddingLeft: bezel.padding.left,
            borderRadius: bezel.outerRadius,
            background: bezel.bezelColor,
          }}
        >
          <div
            aria-hidden
            className="pointer-events-none absolute inset-0"
            style={{
              borderRadius: bezel.outerRadius,
              boxShadow: "inset 0 0 0 1px rgb(255 255 255 / 0.08)",
            }}
          />
          <div
            className="relative bg-background"
            data-testid="playground-device-screen"
            style={{
              width: viewport.width,
              height: viewport.height,
              borderRadius: bezel.innerRadius,
            }}
          >
            {children}
            <div
              aria-hidden
              className="pointer-events-none absolute inset-0"
              data-testid="playground-device-screen-mask"
              style={{
                borderRadius: bezel.innerRadius,
                boxShadow: `0 0 0 ${Math.max(bezel.padding.top, bezel.padding.right, bezel.padding.bottom, bezel.padding.left)}px ${bezel.bezelColor}`,
              }}
            />
          </div>
          <DeviceFaceChrome bezel={bezel} orientation={orientation} />
        </div>
      </div>
    </div>
  );
}

function HardwareNubs({ bezel }: { bezel: PlaygroundDeviceBezel }) {
  return (
    <>
      {bezel.nubs.map((nub) => (
        <div
          aria-hidden
          className="pointer-events-none absolute"
          data-testid={`playground-device-nub-${nub.id}`}
          key={nub.id}
          style={nubStyle(nub, bezel.nubColor)}
        />
      ))}
    </>
  );
}

function nubStyle(nub: PlaygroundBezelNub, color: string): React.CSSProperties {
  const along = `${nub.offsetRatio * 100}%`;
  const base: React.CSSProperties = {
    background: color,
    borderRadius: 1,
  };
  return edgeBox(nub.edge, {
    ...base,
    along,
    length: nub.length,
    thickness: nub.thickness,
    outside: true,
  });
}

function DeviceFaceChrome({
  bezel,
  orientation,
}: {
  bezel: PlaygroundDeviceBezel;
  orientation: "portrait" | "landscape";
}) {
  return (
    <>
      {bezel.island ? (
        <div
          aria-hidden
          className="pointer-events-none absolute z-10"
          data-testid="playground-device-island"
          style={{
            ...chinSlotStyle({
              orientation,
              slot: "start",
              padding: bezel.padding,
              size: bezel.island,
            }),
            background: bezel.faceColor,
            borderRadius: 999,
          }}
        />
      ) : null}
      {bezel.punch ? (
        <div
          aria-hidden
          className="pointer-events-none absolute z-10"
          data-testid="playground-device-punch"
          style={{
            ...chinSlotStyle({
              orientation,
              slot: "start",
              padding: bezel.padding,
              size: bezel.punch,
            }),
            background: bezel.faceColor,
            borderRadius: 999,
          }}
        />
      ) : null}
      {bezel.cameraDot ? (
        <div
          aria-hidden
          className="pointer-events-none absolute z-10"
          data-testid="playground-device-camera"
          style={{
            ...chinSlotStyle({
              orientation,
              slot: "start",
              padding: bezel.padding,
              size: {
                width: bezel.cameraDot.size,
                height: bezel.cameraDot.size,
              },
            }),
            background: bezel.faceColor,
            borderRadius: 999,
          }}
        />
      ) : null}
      {bezel.homeIndicator ? (
        <div
          aria-hidden
          className="pointer-events-none absolute z-10"
          data-testid="playground-device-home-indicator"
          style={{
            ...chinSlotStyle({
              orientation,
              slot: "end",
              padding: bezel.padding,
              size: bezel.homeIndicator,
            }),
            background: bezel.indicatorColor,
            borderRadius: 999,
          }}
        />
      ) : null}
      {bezel.homeButton ? (
        <div
          aria-hidden
          className="pointer-events-none absolute z-10"
          data-testid="playground-device-home-button"
          style={{
            ...chinSlotStyle({
              orientation,
              slot: "end",
              padding: bezel.padding,
              size: {
                width: bezel.homeButton.size,
                height: bezel.homeButton.size,
              },
            }),
            border: `2px solid ${bezel.nubColor}`,
            borderRadius: 999,
            background: bezel.bezelColor,
          }}
        />
      ) : null}
    </>
  );
}

function chinSlotStyle({
  orientation,
  padding,
  size,
  slot,
}: {
  orientation: "portrait" | "landscape";
  padding: PlaygroundDeviceBezel["padding"];
  size: { width: number; height: number };
  slot: "start" | "end";
}): React.CSSProperties {
  const rotated = orientation === "landscape";
  const width = rotated ? size.height : size.width;
  const height = rotated ? size.width : size.height;
  if (!rotated) {
    if (slot === "start") {
      return {
        top: (padding.top - height) / 2,
        left: "50%",
        width,
        height,
        transform: "translateX(-50%)",
      };
    }
    return {
      bottom: (padding.bottom - height) / 2,
      left: "50%",
      width,
      height,
      transform: "translateX(-50%)",
    };
  }
  if (slot === "start") {
    return {
      left: (padding.left - width) / 2,
      top: "50%",
      width,
      height,
      transform: "translateY(-50%)",
    };
  }
  return {
    right: (padding.right - width) / 2,
    top: "50%",
    width,
    height,
    transform: "translateY(-50%)",
  };
}

function edgeBox(
  edge: PlaygroundBezelEdge,
  box: {
    along: string;
    length: number;
    thickness: number;
    outside: boolean;
  } & React.CSSProperties,
): React.CSSProperties {
  const { along, length, thickness, outside, ...rest } = box;
  const inset = outside ? -thickness : 0;
  switch (edge) {
    case "left":
      return {
        ...rest,
        left: inset,
        top: along,
        width: thickness,
        height: length,
      };
    case "right":
      return {
        ...rest,
        right: inset,
        top: along,
        width: thickness,
        height: length,
      };
    case "top":
      return {
        ...rest,
        top: inset,
        left: along,
        width: length,
        height: thickness,
      };
    case "bottom":
      return {
        ...rest,
        bottom: inset,
        left: along,
        width: length,
        height: thickness,
      };
  }
}
