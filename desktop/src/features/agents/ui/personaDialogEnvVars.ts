export function hasText(value: string | null | undefined): boolean {
  return (value?.trim().length ?? 0) > 0;
}
