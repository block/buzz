import { useCallback, useEffect, useState } from "react";
import {
  getTtsPlaybackSpeed,
  setTtsPlaybackSpeed,
} from "@/shared/api/ttsPlayback";
import { Button } from "@/shared/ui/button";
import { createPlaybackSpeedPersistence } from "./playbackSpeedPersistence";
import { SettingsOptionGroup, SettingsOptionRow } from "./SettingsOptionGroup";

const DEFAULT_SPEED = 1;
const SLIDER_MIN_SPEED = 0.5;
const SLIDER_MAX_SPEED = 2;
const INPUT_MIN_SPEED = 0.25;
const INPUT_MAX_SPEED = 4;
const SPEED_STEP = 0.05;
const playbackSpeedPersistence = createPlaybackSpeedPersistence(
  setTtsPlaybackSpeed,
  DEFAULT_SPEED,
);

export function SpeechPlaybackSettings() {
  const [speed, setSpeed] = useState(DEFAULT_SPEED);
  const [speedInput, setSpeedInput] = useState(String(DEFAULT_SPEED));
  const [error, setError] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const setDisplayedSpeed = useCallback((nextSpeed: number) => {
    setSpeed(nextSpeed);
    setSpeedInput(String(nextSpeed));
  }, []);

  useEffect(() => {
    let loadActive = true;
    const unsubscribe = playbackSpeedPersistence.subscribe({
      setError,
      setSpeed: setDisplayedSpeed,
    });
    getTtsPlaybackSpeed()
      .then((savedSpeed) => {
        if (loadActive) {
          playbackSpeedPersistence.applyLoaded(savedSpeed);
        }
      })
      .catch((cause) => {
        if (loadActive) setError(String(cause));
      })
      .finally(() => {
        if (loadActive) setLoaded(true);
      });
    return () => {
      loadActive = false;
      unsubscribe();
    };
  }, [setDisplayedSpeed]);

  const commitSpeed = (nextSpeed: number) => {
    playbackSpeedPersistence.commit(nextSpeed);
  };

  const commitTypedSpeed = () => {
    const nextSpeed = Number(speedInput);
    if (Number.isFinite(nextSpeed)) {
      commitSpeed(nextSpeed);
    } else {
      setSpeedInput(String(speed));
    }
  };

  return (
    <SettingsOptionGroup>
      <SettingsOptionRow className="items-start">
        <div className="min-w-0 flex-1">
          <div className="flex items-center justify-between gap-4">
            <div>
              <label
                className="text-sm font-medium"
                htmlFor="speech-playback-speed"
              >
                Speech playback speed
              </label>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <div className="flex items-center gap-1">
                <input
                  aria-label="Speech playback speed value"
                  className="h-8 w-16 rounded-md border border-input bg-background px-2 text-right text-sm tabular-nums"
                  disabled={!loaded}
                  max={INPUT_MAX_SPEED}
                  min={INPUT_MIN_SPEED}
                  onBlur={commitTypedSpeed}
                  onChange={(event) => setSpeedInput(event.currentTarget.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      commitTypedSpeed();
                      event.currentTarget.blur();
                    }
                  }}
                  step={SPEED_STEP}
                  type="number"
                  value={speedInput}
                />
                <span aria-hidden="true" className="text-sm">
                  ×
                </span>
              </div>
              <Button
                disabled={!loaded || speed === DEFAULT_SPEED}
                onClick={() => commitSpeed(DEFAULT_SPEED)}
                size="sm"
                type="button"
                variant="outline"
              >
                Reset
              </Button>
            </div>
          </div>
          <input
            aria-valuetext={`${speed.toFixed(2)} times`}
            className="mt-3 w-full accent-primary"
            disabled={!loaded}
            id="speech-playback-speed"
            max={SLIDER_MAX_SPEED}
            min={SLIDER_MIN_SPEED}
            onBlur={(event) => commitSpeed(Number(event.currentTarget.value))}
            onChange={(event) =>
              setDisplayedSpeed(Number(event.currentTarget.value))
            }
            onKeyUp={(event) => commitSpeed(Number(event.currentTarget.value))}
            onPointerUp={(event) =>
              commitSpeed(Number(event.currentTarget.value))
            }
            step={SPEED_STEP}
            type="range"
            value={speed}
          />
          {error ? (
            <p className="mt-1 text-xs text-destructive" role="alert">
              Could not save playback speed: {error}
            </p>
          ) : null}
        </div>
      </SettingsOptionRow>
    </SettingsOptionGroup>
  );
}
