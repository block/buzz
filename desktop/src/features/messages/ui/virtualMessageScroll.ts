export function getVirtualMessageScrollOptions(
  behavior: ScrollBehavior | undefined,
) {
  return {
    align: "center" as const,
    smooth: behavior === "smooth",
  };
}
