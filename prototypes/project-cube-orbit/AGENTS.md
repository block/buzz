## Canonical workspace

- This is the canonical Buzz prototype directory: `/Users/cynthiac/Development/buzz/prototypes/project-cube-orbit`.
- Work on branch `cc-experiment` in `/Users/cynthiac/Development/buzz`.
- Before changing prototype files, verify that `git remote get-url origin` is `https://github.com/block/buzz.git` and `git branch --show-current` is `cc-experiment`.
- Do not copy or mirror implementation changes into `/Users/cynthiac/Documents/Buzz`; that directory is a legacy duplicate.
- Run and link the local prototype at exactly `http://127.0.0.1:5173/`.

## UI consistency

- Before creating a component or adding styles, search for an existing component, variant, token, or pattern that can be reused or extended.
- Prefer shared components and design tokens over one-off markup, duplicated declarations, or near-identical variants.
- When a UI pattern repeats, extract the smallest reusable abstraction that preserves accessibility and existing behavior.
- Keep feature-specific layout and behavior with the feature; move broadly reusable controls, surfaces, and typography patterns into the shared UI layer.
- Use regular font weight for all body copy and container headlines unless the user explicitly specifies another weight.
- If it is unclear whether an existing pattern should be reused, extended, or replaced, ask the user before introducing a competing component or styling approach.
