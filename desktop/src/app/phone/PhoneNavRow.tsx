// Phone nav row ([data-phone] only): the same Back / Forward / Home controls desktop and iPad have in their top chrome
// (AppTopChrome), driven by the same router history (useBackForwardControls), with no destination labels. It is one
// fixed 40px row under the safe area on every phone screen (channel, Inbox, Channels/Me/Search lists, Settings), so the
// floating tab-count pill (PhoneTabSwitcher) sits at its right end and never needs a row of its own.
import { ChevronLeft, ChevronRight, House } from "lucide-react";

export function PhoneNavRow({
  canGoBack,
  canGoForward,
  onGoBack,
  onGoForward,
  onHome,
}: {
  canGoBack: boolean;
  canGoForward: boolean;
  onGoBack: () => void;
  onGoForward: () => void;
  onHome: () => void;
}) {
  return (
    <nav aria-label="Navigation" className="phone-nav-row" data-testid="phone-nav-row">
      <button
        aria-label="Go back"
        className="phone-nav-button"
        data-testid="phone-nav-back"
        disabled={!canGoBack}
        onClick={onGoBack}
        type="button"
      >
        <ChevronLeft aria-hidden="true" />
      </button>
      <button
        aria-label="Go forward"
        className="phone-nav-button"
        data-testid="phone-nav-forward"
        disabled={!canGoForward}
        onClick={onGoForward}
        type="button"
      >
        <ChevronRight aria-hidden="true" />
      </button>
      <button
        aria-label="Home (Channels)"
        className="phone-nav-button"
        data-testid="phone-nav-home"
        onClick={onHome}
        type="button"
      >
        <House aria-hidden="true" />
      </button>
    </nav>
  );
}
