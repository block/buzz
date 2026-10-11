# Web vs desktop: JM's decisions (running list)

1. **Pinned sites do not exist on web/mobile.** Removed from Settings, the left-nav group and the `/pins/*` route (gated with `isTauri()`); desktop unchanged. The bridge "Desktop browser" panel (slice R) stays. *(built)*
2. **Mobile split panes (queued).** One shared stacked-split component with a single breakpoint: below it, list only → detail pushed full-screen on selection, with a visible Back button in the header at each layer plus swipe-back; list keeps scroll position and selection; deep links open the detail directly and Back returns to the list; above it, the existing two-pane layout (tablets/landscape stay two-pane if wide enough). First users: Inbox and Projects; more views to come.
3. **Permanent user bar on mobile (queued).** Below the breakpoint the user row (avatar, name…) becomes a bar fixed at the bottom of every screen, including stacked detail views and settings; respects `env(safe-area-inset-bottom)`, never covers composers/inputs, handles the iOS keyboard; same component, same tap behavior. Desktop-style web layout unchanged.
4. **My agents must look the same at desktop and mobile widths (queued).**
