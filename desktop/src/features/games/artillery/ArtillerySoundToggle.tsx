import { Volume2, VolumeX } from "lucide-react";
import * as React from "react";

import {
  isArtilleryAudioEnabled,
  setArtilleryAudioEnabled,
  unlockArtilleryAudio,
} from "@/features/games/artillery/artilleryAudio";
import { Button } from "@/shared/ui/button";

export function ArtillerySoundToggle() {
  const [enabled, setEnabled] = React.useState(isArtilleryAudioEnabled);

  React.useEffect(() => {
    const unlock = () => void unlockArtilleryAudio();
    window.addEventListener("pointerdown", unlock, {
      capture: true,
      once: true,
    });
    return () => window.removeEventListener("pointerdown", unlock, true);
  }, []);

  const toggle = () => {
    const nextEnabled = !enabled;
    setEnabled(nextEnabled);
    setArtilleryAudioEnabled(nextEnabled);
  };

  return (
    <Button
      aria-pressed={enabled}
      data-testid="artillery-sound-toggle"
      onClick={toggle}
      type="button"
      variant="outline"
    >
      {enabled ? (
        <Volume2 aria-hidden="true" />
      ) : (
        <VolumeX aria-hidden="true" />
      )}
      {enabled ? "Sound on" : "Sound off"}
    </Button>
  );
}
