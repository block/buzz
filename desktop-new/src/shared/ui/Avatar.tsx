import { Avatar as BaseAvatar } from "@base-ui/react/avatar";

type AvatarSize = "small" | "default" | "large";

export function Avatar({
  src,
  alt,
  fallback,
  size = "default",
}: {
  src?: string | null;
  alt: string;
  fallback: string;
  size?: AvatarSize;
}) {
  return (
    <BaseAvatar.Root className="buzz-avatar" data-size={size}>
      {src ? <BaseAvatar.Image src={src} alt={alt} /> : null}
      <BaseAvatar.Fallback delay={src ? 150 : 0}>
        {fallback.slice(0, 1).toUpperCase()}
      </BaseAvatar.Fallback>
    </BaseAvatar.Root>
  );
}
