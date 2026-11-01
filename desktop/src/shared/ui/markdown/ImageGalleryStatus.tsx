// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
type ImageGalleryStatusProps = {
  currentIndex: number;
  itemCount: number;
};

export function ImageGalleryStatus({
  currentIndex,
  itemCount,
}: ImageGalleryStatusProps) {
  if (itemCount <= 1) {
    return null;
  }

  const position = currentIndex + 1;
  return (
    <>
      <div
        aria-hidden="true"
        className="h-5 w-px shrink-0 bg-muted-foreground/15"
      />
      <span
        aria-label={uiT("Image {0} of {1}", { p0: position, p1: itemCount })}
        aria-live="polite"
        className="min-w-9 text-center text-xs font-medium tabular-nums text-muted-foreground"
        role="status"
      >
        {position} / {itemCount}
      </span>
    </>
  );
}
