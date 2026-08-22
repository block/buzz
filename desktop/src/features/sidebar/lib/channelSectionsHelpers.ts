import type { ChannelSectionStore } from "./channelSectionsStorage";

export function swapSectionOrder(
  prev: ChannelSectionStore,
  sectionId: string,
  direction: "up" | "down",
): ChannelSectionStore | null {
  const target = prev.sections.find((s) => s.id === sectionId);
  if (!target) return null;
  const sorted = prev.sections.slice().sort((a, b) => a.order - b.order);
  const idx = sorted.findIndex((s) => s.id === sectionId);
  const neighborIdx = direction === "up" ? idx - 1 : idx + 1;
  if (neighborIdx < 0 || neighborIdx >= sorted.length) return null;
  const neighbor = sorted[neighborIdx];
  const sections = prev.sections.map((s) => {
    if (s.id === target.id) return { ...s, order: neighbor.order };
    if (s.id === neighbor.id) return { ...s, order: target.order };
    return s;
  });
  return { ...prev, sections };
}
