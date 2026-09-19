
import {
  PLAYGROUND_DEVICES,
  PLAYGROUND_DEVICE_SCALE_DEFAULT,
  PLAYGROUND_DEVICE_SCALE_MAX,
  PLAYGROUND_DEVICE_SCALE_MIN,
  stepPlaygroundDeviceScale,
  type PlaygroundDeviceId,
  type PlaygroundDeviceScalePercent,
} from "../lib/devices";

export type DeviceMuseumOrientation = "portrait" | "landscape";

/**
 * Shared Mobile museum controls for playground stage + link/pin slide-out.
 * Device select, portrait↔landscape, and 50–200% scale in 25% steps.
 */
export function DeviceMuseumToolbar({
  deviceId,
  onDeviceIdChange,
  orientation,
  onOrientationToggle,
  scalePercent,
  onScalePercentChange,
  testIdPrefix,
}: {
  deviceId: PlaygroundDeviceId;
  onDeviceIdChange: (id: PlaygroundDeviceId) => void;
  orientation: DeviceMuseumOrientation;
  onOrientationToggle: () => void;
  scalePercent: PlaygroundDeviceScalePercent;
  onScalePercentChange: (percent: PlaygroundDeviceScalePercent) => void;
  /** e.g. "playground" or "link-side-panel" */
  testIdPrefix: string;
}) {
  const scale = scalePercent || PLAYGROUND_DEVICE_SCALE_DEFAULT;
  return (
    <div className="flex flex-wrap items-center gap-2 px-3">
      <select
        className="rounded-md border border-border bg-background px-2 py-1 text-xs"
        data-testid={`${testIdPrefix}-device-select`}
        onChange={(event) =>
          onDeviceIdChange(event.target.value as PlaygroundDeviceId)
        }
        value={deviceId}
      >
        {PLAYGROUND_DEVICES.map((item) => (
          <option
            data-testid={`${testIdPrefix}-device-${item.id}`}
            key={item.id}
            value={item.id}
          >
            {item.name}
          </option>
        ))}
      </select>
      <button
        className="rounded-md border border-border px-2 py-1 text-xs"
        data-testid={`${testIdPrefix}-orientation`}
        onClick={onOrientationToggle}
        type="button"
      >
        {orientation === "portrait" ? "Portrait" : "Landscape"}
      </button>
      <div
        className="flex items-center gap-1"
        data-testid={`${testIdPrefix}-device-scale`}
      >
        <button
          aria-label="Decrease device scale"
          className="rounded-md border border-border px-2 py-1 text-xs disabled:opacity-40"
          data-testid={`${testIdPrefix}-device-scale-down`}
          disabled={scale <= PLAYGROUND_DEVICE_SCALE_MIN}
          onClick={() => onScalePercentChange(stepPlaygroundDeviceScale(scale, -1))}
          type="button"
        >
          −
        </button>
        <span
          className="min-w-12 text-center text-xs tabular-nums text-foreground"
          data-testid={`${testIdPrefix}-device-scale-value`}
        >
          {scale}%
        </span>
        <button
          aria-label="Increase device scale"
          className="rounded-md border border-border px-2 py-1 text-xs disabled:opacity-40"
          data-testid={`${testIdPrefix}-device-scale-up`}
          disabled={scale >= PLAYGROUND_DEVICE_SCALE_MAX}
          onClick={() => onScalePercentChange(stepPlaygroundDeviceScale(scale, 1))}
          type="button"
        >
          +
        </button>
      </div>
    </div>
  );
}
