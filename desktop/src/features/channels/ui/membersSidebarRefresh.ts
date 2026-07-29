export function shouldRefreshMembersOnOpen(open: boolean, wasOpen: boolean) {
  return open && !wasOpen;
}
