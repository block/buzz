export const GROUP_HANDLE_PATTERN = /^[a-z0-9][a-z0-9_-]{1,31}$/;

export function isValidGroupHandle(handle: string): boolean {
  return GROUP_HANDLE_PATTERN.test(handle);
}
