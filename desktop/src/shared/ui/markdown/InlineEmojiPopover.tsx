import * as React from "react";

import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";

export function InlineEmojiPopover({
  alt,
  resolvedSrc,
}: {
  alt: string | undefined;
  resolvedSrc: string;
}) {
  const [open, setOpen] = React.useState(false);
  const openTimeout = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const closeTimeout = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const label = alt?.trim() || "Custom emoji";

  const clearTimers = React.useCallback(() => {
    if (openTimeout.current) {
      clearTimeout(openTimeout.current);
      openTimeout.current = null;
    }
    if (closeTimeout.current) {
      clearTimeout(closeTimeout.current);
      closeTimeout.current = null;
    }
  }, []);

  const handleMouseEnter = React.useCallback(() => {
    clearTimers();
    openTimeout.current = setTimeout(() => setOpen(true), 200);
  }, [clearTimers]);

  const scheduleClose = React.useCallback(() => {
    clearTimers();
    closeTimeout.current = setTimeout(() => setOpen(false), 150);
  }, [clearTimers]);

  const handleFocus = React.useCallback(() => {
    clearTimers();
    setOpen(true);
  }, [clearTimers]);

  React.useEffect(() => clearTimers, [clearTimers]);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className="inline-flex border-0 bg-transparent p-0 align-middle text-inherit"
          aria-label={label}
          onMouseEnter={handleMouseEnter}
          onMouseLeave={scheduleClose}
          onFocus={handleFocus}
          onBlur={scheduleClose}
        >
          <img
            alt={alt}
            title={label}
            src={resolvedSrc}
            data-custom-emoji=""
            className="mx-px inline-block h-[1.25em] w-auto max-w-none align-middle"
            draggable={false}
            onContextMenu={(e) => e.preventDefault()}
          />
        </button>
      </PopoverTrigger>
      <PopoverContent
        align="center"
        side="top"
        sideOffset={6}
        className="w-auto min-w-32 max-w-56 rounded-xl p-3"
        onMouseEnter={handleMouseEnter}
        onMouseLeave={scheduleClose}
        onOpenAutoFocus={(e) => e.preventDefault()}
        onCloseAutoFocus={(e) => e.preventDefault()}
      >
        <div className="flex flex-col items-center text-center">
          <div className="mb-2 flex h-14 w-14 items-center justify-center">
            <img
              alt={alt}
              src={resolvedSrc}
              className="inline-block h-12 w-12 object-contain"
              draggable={false}
            />
          </div>
          <div className="max-w-[12rem] text-balance text-sm font-semibold leading-snug text-popover-foreground">
            {label}
          </div>
        </div>
      </PopoverContent>
    </Popover>
  );
}
