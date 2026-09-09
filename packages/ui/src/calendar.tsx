import { DayPicker, type DayPickerProps } from "@daypicker/react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { cx } from "./classes";
/** A keyboard-accessible calendar; selection, locale, and disabled dates use DayPicker's API. */
export function Calendar({ className, components, ...props }: DayPickerProps) {
  return (
    <DayPicker
      {...props}
      className={cx("bui-calendar", className)}
      components={{
        Chevron: ({ orientation }) =>
          orientation === "left" ? (
            <ChevronLeft aria-hidden="true" />
          ) : (
            <ChevronRight aria-hidden="true" />
          ),
        ...components,
      }}
    />
  );
}
