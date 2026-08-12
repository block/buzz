export const CHANNEL_SWITCH_START_MARK = "buzz:channel-switch:start";
export const CHANNEL_ROWS_PAINTED_MARK = "buzz:channel-switch:rows-painted";
export const CHANNEL_SWITCH_MEASURE =
  "buzz:channel-switch:rows-painted-duration";

export const THREAD_OPEN_START_MARK = "buzz:thread-open:start";
export const THREAD_REPLIES_PAINTED_MARK = "buzz:thread-open:replies-painted";
export const THREAD_OPEN_MEASURE = "buzz:thread-open:replies-painted-duration";

export function startPerformanceMark(name: string): void {
  if (typeof performance === "undefined") return;
  performance.clearMarks(name);
  performance.mark(name);
}

export function finishPerformanceMeasure(input: {
  startMark: string;
  endMark: string;
  measure: string;
}): void {
  if (
    typeof performance === "undefined" ||
    performance.getEntriesByName(input.startMark, "mark").length === 0
  ) {
    return;
  }
  performance.clearMarks(input.endMark);
  performance.mark(input.endMark);
  performance.clearMeasures(input.measure);
  performance.measure(input.measure, input.startMark, input.endMark);
}
