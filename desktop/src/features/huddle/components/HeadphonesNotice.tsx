import { Headphones } from "lucide-react";

/**
 * "Headphones recommended — echo cancellation lands in the next release."
 *
 * Session-dismissable banner shown on the active HuddleBar while the desktop
 * client lacks an echo-cancellation reference. The current play path is
 * native rodio (outside the WebView render graph), so the browser's WebRTC
 * AEC cannot suppress local-speaker → mic feedback for users on speakers.
 *
 * The follow-up PR moves remote-peer playout into WebAudio inside the same
 * `AudioContext` as `getUserMedia({ echoCancellation: true })`. When that
 * lands, the parent component flips `aecMissing` to false and this banner
 * (and this file) become removable in a single diff.
 *
 * State (`dismissed`, `aecMissing`) lives in the parent so the parent can
 * decide whether to render at all. Keep this component dumb so the
 * deletion later is mechanical.
 */
export function HeadphonesNotice({ onDismiss }: { onDismiss: () => void }) {
  return (
    <div
      role="status"
      data-testid="huddle-headphones-notice"
      className="flex items-center gap-1.5 rounded bg-amber-500/10 px-2 py-1 text-xs text-amber-700 dark:text-amber-300"
    >
      <Headphones className="h-3 w-3" />
      <span className="max-w-[260px] truncate">
        Headphones recommended — echo cancellation lands in the next release.
      </span>
      <button
        aria-label="Dismiss headphones notice for this session"
        className="ml-1 opacity-60 hover:opacity-100"
        onClick={onDismiss}
        type="button"
      >
        ✕
      </button>
    </div>
  );
}
