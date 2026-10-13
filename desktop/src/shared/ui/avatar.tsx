import * as React from "react";
import * as AvatarPrimitive from "@radix-ui/react-avatar";

import { cn } from "@/shared/lib/cn";
import { useRelayMediaSrc } from "@/shared/lib/useRelayMediaSrc";

const Avatar = React.forwardRef<
  React.ElementRef<typeof AvatarPrimitive.Root>,
  React.ComponentPropsWithoutRef<typeof AvatarPrimitive.Root>
>(({ className, ...props }, ref) => (
  <AvatarPrimitive.Root
    ref={ref}
    className={cn(
      "relative flex h-9 w-9 shrink-0 overflow-hidden rounded-full",
      className,
    )}
    {...props}
  />
));
Avatar.displayName = AvatarPrimitive.Root.displayName;

const AvatarImage = React.forwardRef<
  React.ElementRef<typeof AvatarPrimitive.Image>,
  React.ComponentPropsWithoutRef<typeof AvatarPrimitive.Image>
>(({ className, src, onLoadingStatusChange, ...props }, ref) => {
  // Web: relay-hosted avatars (/media/<sha>) need a signed Authorization header a plain <img> can't send, so they are
  // fetched into a blob: URL first (desktop's localhost proxy does this natively; the hook is identity there).
  const media = useRelayMediaSrc(src);
  // While the signed fetch is in flight the image has no src yet, and Radix reports that as "error". Callers that
  // remember failures (ProfileAvatar: DM rows, profile panel) then stuck on initials forever. Only forward an error
  // once the media fetch itself failed or the real URL failed to load.
  const pending = media.state === "loading" || media.state === "idle";
  const failed = media.state === "error";
  const statusCb = React.useRef(onLoadingStatusChange);
  statusCb.current = onLoadingStatusChange;
  React.useEffect(() => {
    if (failed) statusCb.current?.("error");
  }, [failed]);
  return (
    <AvatarPrimitive.Image
      ref={ref}
      className={cn("aspect-square h-full w-full avatar-sdr-clamp", className)}
      decoding="async"
      loading="lazy"
      {...props}
      onLoadingStatusChange={(status) => {
        if (status === "error" && (pending || failed)) return;
        onLoadingStatusChange?.(status);
      }}
      src={media.src}
    />
  );
});
AvatarImage.displayName = AvatarPrimitive.Image.displayName;

const AvatarFallback = React.forwardRef<
  React.ElementRef<typeof AvatarPrimitive.Fallback>,
  React.ComponentPropsWithoutRef<typeof AvatarPrimitive.Fallback>
>(({ className, ...props }, ref) => (
  <AvatarPrimitive.Fallback
    ref={ref}
    className={cn(
      "flex h-full w-full items-center justify-center rounded-[inherit] bg-muted",
      className,
    )}
    {...props}
  />
));
AvatarFallback.displayName = AvatarPrimitive.Fallback.displayName;

export { Avatar, AvatarImage, AvatarFallback };
