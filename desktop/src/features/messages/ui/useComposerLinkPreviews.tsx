import * as React from "react";

import { extractSupportedLinkPreviews } from "@/shared/lib/linkPreview";
import { useResolvedLinkPreviews } from "@/shared/lib/useResolvedLinkPreviews";
import { LinkPreviewList } from "@/shared/ui/link-preview-list";
import { LinkPreviewImageLightbox } from "@/shared/ui/markdown";

export function useComposerLinkPreviews() {
  const [suppressedUrls, setSuppressedUrls] = React.useState<Set<string>>(
    () => new Set(),
  );
  const suppressedUrlsRef = React.useRef(suppressedUrls);
  suppressedUrlsRef.current = suppressedUrls;
  const [content, setContent] = React.useState("");
  const candidates = React.useMemo(
    () =>
      extractSupportedLinkPreviews(content).filter(
        (preview) => !suppressedUrls.has(preview.href),
      ),
    [content, suppressedUrls],
  );
  const previews = useResolvedLinkPreviews(candidates);
  const removePreview = React.useCallback((href: string) => {
    setSuppressedUrls((current) => new Set(current).add(href));
  }, []);
  const previewList = previews.length ? (
    <div className="message-markdown mb-2" data-composer-link-previews="">
      <LinkPreviewList
        ImageLightbox={LinkPreviewImageLightbox}
        onRemovePreview={removePreview}
        previews={previews}
      />
    </div>
  ) : null;

  return {
    previewList,
    setPreviewContent: setContent,
    setSuppressedLinkPreviewUrls: setSuppressedUrls,
    suppressedLinkPreviewUrls: suppressedUrls,
    suppressedLinkPreviewUrlsRef: suppressedUrlsRef,
  };
}
