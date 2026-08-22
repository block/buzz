export function VideoReviewPosterPreview({
  poster,
  visible,
}: {
  poster?: string;
  visible: boolean;
}) {
  if (!poster || !visible) return null;

  return (
    <img
      alt=""
      aria-hidden="true"
      className="pointer-events-none absolute inset-0 h-full w-full object-contain"
      data-testid="video-review-poster-preview"
      decoding="sync"
      draggable={false}
      fetchPriority="high"
      src={poster}
    />
  );
}
