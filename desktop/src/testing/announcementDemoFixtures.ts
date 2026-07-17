export type AnnouncementDemoPersonKey =
  | "viewer"
  | "engineer"
  | "designer"
  | "marketing"
  | "researcher"
  | "qa"
  | "data"
  | "support"
  | "producer"
  | "community";

export type AnnouncementDemoChannelRole =
  | "owner"
  | "admin"
  | "member"
  | "guest";

export const ANNOUNCEMENT_DEMO_COMMUNITY_NAME = "Honeycomb Studios";

export const ANNOUNCEMENT_DEMO_AGENTS = [
  {
    pubkey: "70".repeat(32),
    name: "Fizz",
    avatarUrl: "/demo/agents/fizz.png",
    systemPrompt:
      "You are Fizz, an energetic maker who turns ideas into action. Be upbeat, practical, and decisive. Help users plan, create, solve problems, and finish work. Add occasional bee wordplay or 🐝✨—keep it charming, never distracting.",
    channelNames: [
      "flight-path",
      "engineering",
      "design",
      "marketing",
      "queen-bee-launch",
    ],
  },
  {
    pubkey: "71".repeat(32),
    name: "Honey",
    avatarUrl: "/demo/agents/honey.png",
    systemPrompt:
      "You are Honey, a warm and thoughtful communicator. Help users write clearly, organize ideas, brainstorm, summarize, and prepare for conversations. Be kind, creative, and concise. Add occasional bee wordplay or 🍯🐝—keep it sweet, never excessive.",
    channelNames: [
      "flight-path",
      "engineering",
      "design",
      "marketing",
      "queen-bee-launch",
    ],
  },
  {
    pubkey: "72".repeat(32),
    name: "Bumble",
    avatarUrl: "/demo/agents/bumble.png",
    systemPrompt:
      "You are Bumble, a curious and adventurous researcher. Explore questions, compare options, check assumptions, and explain what you find clearly. Be candid when uncertain and favor useful evidence. Add occasional bee wordplay or 🐝🔎—keep it playful, never chaotic.",
    channelNames: [
      "flight-path",
      "engineering",
      "design",
      "marketing",
      "queen-bee-launch",
    ],
  },
] as const;

export type AnnouncementDemoAgentName =
  (typeof ANNOUNCEMENT_DEMO_AGENTS)[number]["name"];

/**
 * Cross-channel cues for the short announcement film. Opening the trigger
 * channel starts real observer-derived turns in the background channels, then
 * delivers a live mention in the pivot channel. The mention is also inserted
 * into the Home feed so the production notification path owns the native
 * notification and click-through behavior.
 */
export const ANNOUNCEMENT_DEMO_STORY = {
  triggerChannelName: "flight-path",
  pivot: {
    delayMs: 8_000,
    channelName: "engineering",
    author: "engineer",
    content:
      "@Alex Rivera — before this prototype gets any bigger, we should talk stack. We’re hand-rolling the iOS and Android views separately and it’s already drifting.",
  },
  engineeringSequence: {
    insight: {
      delayMs: 2_500,
      author: "data",
      content:
        "Honestly? Move it to Flutter now. One codebase, both platforms, and we stop maintaining two of everything. Cheap to switch today, expensive in a month.",
    },
    retask: {
      delayMs: 4_000,
      content:
        "@Bumble New direction, team — we’re moving the prototype to Flutter. Can you and the others re-plan the port and start scaffolding?",
    },
    agentReplies: {
      Bumble:
        "🫡 On it — mapping the current React views to Flutter widgets now. Handing the state layer to @Fizz.",
      Fizz: "Got the state layer — porting the game loop to Flutter, keeping the tuned physics. @Honey, UI shell?",
      Honey:
        "Building the Flutter widget tree from the existing designs — same look, one codebase. 🍯",
    },
    humanClose: {
      delayMs: 1_100,
      author: "data",
      content:
        "That’s the version. One stack, both platforms — nice pivot, team. 🐝",
    },
  },
  workingTurns: [
    { channelName: "design", agent: "Honey" },
    { channelName: "engineering", agent: "Bumble" },
    { channelName: "marketing", agent: "Fizz" },
  ],
} as const;

export const ANNOUNCEMENT_DEMO_LIVE_CONVERSATIONS = [
  {
    channelName: "flight-path",
    agentOrder: ["Fizz", "Honey", "Bumble"],
    humanClose: {
      delayMs: 1_000,
      author: "producer",
      content: "That’s the version. I can cut to that — nice swarm work 🐝",
    },
    steps: [
      {
        delayMs: 2_400,
        author: "engineer",
        content:
          "Small thing: the desktop-to-mobile handoff still feels a little fast.",
        agentMentions: [],
        startsAgentChain: false,
      },
      {
        delayMs: 2_800,
        author: "designer",
        content: "Yeah — I want one extra beat on the sent message.",
        agentMentions: [],
        startsAgentChain: false,
      },
      {
        delayMs: 2_600,
        author: "producer",
        content: "That would give the camera somewhere to land too.",
        agentMentions: [],
        startsAgentChain: false,
      },
      {
        delayMs: 2_800,
        author: "engineer",
        content:
          "@Fizz can you turn that into a clean three-beat capture plan?",
        agentMentions: ["Fizz"],
        startsAgentChain: true,
      },
    ],
  },
  {
    channelName: "engineering",
    agentOrder: ["Bumble", "Fizz", "Honey"],
    humanClose: {
      delayMs: 1_100,
      author: "engineer",
      content:
        "Perfect. That gives me the patch and the verification path — I’m on it.",
    },
    steps: [
      {
        delayMs: 2_600,
        author: "qa",
        content:
          "I can still reproduce one duplicate message after waking the laptop.",
        agentMentions: [],
        startsAgentChain: false,
      },
      {
        delayMs: 3_000,
        author: "engineer",
        content: "Only on the first reconnect?",
        agentMentions: [],
        startsAgentChain: false,
      },
      {
        delayMs: 2_800,
        author: "qa",
        content: "Yep. One duplicate, then the subscription settles.",
        agentMentions: [],
        startsAgentChain: false,
      },
      {
        delayMs: 3_000,
        author: "engineer",
        content:
          "@Bumble can you trace the likely path and pull Fizz and Honey into a fix plan?",
        agentMentions: ["Bumble"],
        startsAgentChain: true,
      },
    ],
  },
] as const;

export const ANNOUNCEMENT_DEMO_PEOPLE = {
  viewer: {
    displayName: "Alex Rivera",
    role: "Product Lead",
    avatarUrl: "/demo/avatars/alex-rivera-candid.png",
    nip05Handle: "alex@honeycomb.studio",
    presence: "online",
  },
  engineer: {
    displayName: "Maya Chen",
    role: "Software Engineer",
    avatarUrl: "/demo/avatars/maya-chen-candid.png",
    nip05Handle: "maya@honeycomb.studio",
    presence: "online",
  },
  designer: {
    displayName: "Jordan Brooks",
    role: "Product Designer",
    avatarUrl: "/demo/avatars/jordan-brooks-candid.png",
    nip05Handle: "jordan@honeycomb.studio",
    presence: "online",
  },
  marketing: {
    displayName: "Priya Shah",
    role: "Marketing Lead",
    avatarUrl: "/demo/avatars/priya-shah-candid.png",
    nip05Handle: "priya@honeycomb.studio",
    presence: "away",
  },
  researcher: {
    displayName: "Sofia Patel",
    role: "User Researcher",
    avatarUrl: "/demo/avatars/sofia-patel-full.png",
    nip05Handle: "sofia@honeycomb.studio",
    presence: "online",
  },
  qa: {
    displayName: "Marcus Reed",
    role: "QA Engineer",
    avatarUrl: "/demo/avatars/marcus-reed-candid.png",
    nip05Handle: "marcus@honeycomb.studio",
    presence: "online",
  },
  data: {
    displayName: "Elena Torres",
    role: "Data Analyst",
    avatarUrl: "/demo/avatars/elena-torres.png",
    nip05Handle: "elena@honeycomb.studio",
    presence: "online",
  },
  support: {
    displayName: "Theo Martin",
    role: "Customer Experience",
    avatarUrl: "/demo/avatars/theo-martin-full.png",
    nip05Handle: "theo@honeycomb.studio",
    presence: "away",
  },
  producer: {
    displayName: "Camille Dubois",
    role: "Video Producer",
    avatarUrl: "/demo/avatars/camille-dubois-candid.png",
    nip05Handle: "camille@honeycomb.studio",
    presence: "online",
  },
  community: {
    displayName: "Noah Kim",
    role: "Community Manager",
    avatarUrl: "/demo/avatars/noah-kim-candid.png",
    nip05Handle: "noah@honeycomb.studio",
    presence: "online",
  },
} as const;

export const ANNOUNCEMENT_DEMO_CHANNELS = [
  {
    id: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
    name: "announcements",
    channelType: "stream",
    visibility: "open",
    description: "Company news, milestones, and important updates",
    topic: "The latest from across Honeycomb Studios",
    purpose: "Keep the whole team aligned on what matters now.",
    createdBy: "viewer",
    lastMessageMinutesAgo: 4,
    memberRoles: [
      ["viewer", "owner"],
      ["engineer", "admin"],
      ["designer", "member"],
      ["marketing", "member"],
      ["researcher", "member"],
      ["qa", "member"],
      ["producer", "member"],
      ["community", "member"],
    ],
  },
  {
    id: "9dae0116-799b-5071-a0a8-fdd30a91a35d",
    name: "general",
    channelType: "stream",
    visibility: "open",
    description: "Quick questions, celebrations, and team chatter",
    topic: "Where everyone checks in",
    purpose: "Make everyday collaboration feel easy and human.",
    createdBy: "viewer",
    lastMessageMinutesAgo: 18,
    memberRoles: [
      ["viewer", "owner"],
      ["engineer", "member"],
      ["designer", "member"],
      ["marketing", "member"],
      ["researcher", "member"],
      ["qa", "member"],
      ["data", "member"],
      ["support", "member"],
      ["producer", "member"],
      ["community", "member"],
    ],
  },
  {
    id: "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9",
    name: "flight-path",
    channelType: "stream",
    visibility: "open",
    description: "Engineering plans for the next Buzz release",
    topic: "Release candidate polish",
    purpose: "Track implementation, quality, and release readiness.",
    createdBy: "engineer",
    lastMessageMinutesAgo: 7,
    memberRoles: [
      ["engineer", "owner"],
      ["viewer", "admin"],
      ["designer", "member"],
      ["qa", "member"],
      ["data", "member"],
      ["producer", "member"],
    ],
  },
  {
    id: "4e91b4d2-8207-4d63-9b1a-3c72a8f01142",
    name: "engineering",
    channelType: "stream",
    visibility: "open",
    description: "Desktop, relay, and infrastructure engineering",
    topic: "Reconnect reliability and the next release candidate",
    purpose: "Debug together, review changes, and keep the build healthy.",
    createdBy: "engineer",
    lastMessageMinutesAgo: 5,
    memberRoles: [
      ["engineer", "owner"],
      ["viewer", "admin"],
      ["qa", "member"],
      ["data", "member"],
      ["support", "member"],
      ["designer", "member"],
    ],
  },
  {
    id: "b5e2f8a1-3c44-5912-9e67-4a8d1f2b3c4e",
    name: "design",
    channelType: "stream",
    visibility: "open",
    description: "Product design, motion, and the Comb Kit system",
    topic: "Announcement film UI pass",
    purpose: "Shape a coherent product experience across every surface.",
    createdBy: "designer",
    lastMessageMinutesAgo: 8,
    memberRoles: [
      ["designer", "owner"],
      ["viewer", "admin"],
      ["engineer", "member"],
      ["researcher", "member"],
      ["producer", "member"],
    ],
  },
  {
    id: "94a444a4-c0a3-5966-ab05-530c6ddc2301",
    name: "mobile",
    channelType: "stream",
    visibility: "open",
    description: "The mobile experience and cross-device continuity",
    topic: "Conversation handoff and mobile polish",
    purpose: "Make Buzz feel continuous wherever the team works.",
    createdBy: "engineer",
    lastMessageMinutesAgo: 26,
    memberRoles: [
      ["engineer", "owner"],
      ["viewer", "member"],
      ["designer", "member"],
      ["qa", "member"],
      ["data", "member"],
      ["support", "member"],
    ],
  },
  {
    id: "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11",
    name: "product-ideas",
    channelType: "forum",
    visibility: "open",
    description: "Research notes and ideas worth growing",
    topic: "Signals from customers and the community",
    purpose: "Turn observations into thoughtful product bets.",
    createdBy: "viewer",
    lastMessageMinutesAgo: 34,
    memberRoles: [
      ["viewer", "owner"],
      ["engineer", "member"],
      ["designer", "member"],
      ["marketing", "member"],
      ["researcher", "member"],
      ["data", "member"],
      ["support", "member"],
      ["community", "member"],
    ],
  },
  {
    id: "c6f3a9b2-4d55-5a23-bf78-5b9e2g3c5d6f",
    name: "marketing",
    channelType: "stream",
    visibility: "open",
    description: "Launch story, social, press, and community rollout",
    topic: "Announcement week",
    purpose: "Carry the Buzz story to every audience with clarity.",
    createdBy: "marketing",
    lastMessageMinutesAgo: 9,
    memberRoles: [
      ["marketing", "owner"],
      ["viewer", "admin"],
      ["designer", "member"],
      ["researcher", "member"],
      ["data", "member"],
      ["producer", "member"],
      ["community", "member"],
    ],
  },
  {
    id: "1be1dcdb-4c31-5a8c-81de-ac102552ca10",
    name: "launch-notes",
    channelType: "forum",
    visibility: "open",
    description: "Final launch decisions, scripts, and review threads",
    topic: "The announcement film",
    purpose: "Keep final feedback focused and easy to find.",
    createdBy: "marketing",
    lastMessageMinutesAgo: 22,
    memberRoles: [
      ["marketing", "owner"],
      ["viewer", "admin"],
      ["engineer", "member"],
      ["designer", "member"],
      ["qa", "member"],
      ["producer", "member"],
    ],
  },
  {
    id: "3c2d9f0a-1b44-5e77-9a21-6f8b0c4d2e91",
    name: "queen-bee-launch",
    channelType: "stream",
    visibility: "private",
    description: "Private launch room for the announcement team",
    topic: "Final cut and release timing",
    purpose: "Coordinate the last mile before Buzz takes flight.",
    createdBy: "viewer",
    lastMessageMinutesAgo: 3,
    memberRoles: [
      ["viewer", "owner"],
      ["engineer", "member"],
      ["designer", "member"],
      ["marketing", "member"],
      ["qa", "member"],
      ["producer", "member"],
      ["community", "member"],
    ],
  },
] as const;

export const ANNOUNCEMENT_DEMO_DMS = [
  {
    id: "f48efb06-0c93-5025-aac9-2e646bb6bfa8",
    person: "engineer",
    lastMessageMinutesAgo: 11,
  },
  {
    id: "7eb9f239-9393-50b0-bd76-d85eef0511c7",
    person: "designer",
    lastMessageMinutesAgo: 16,
  },
  {
    id: "d1ec7000-d000-4000-8000-000000000001",
    person: "marketing",
    lastMessageMinutesAgo: 6,
  },
] as const;

// Start the recording workspace with a believable mix of read and unread
// conversations. Channels omitted from this list retain their unread state.
export const ANNOUNCEMENT_DEMO_INITIAL_READ_CHANNEL_IDS = [
  "9dae0116-799b-5071-a0a8-fdd30a91a35d", // general
  "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9", // flight-path
  "94a444a4-c0a3-5966-ab05-530c6ddc2301", // mobile
  "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11", // product-ideas
  "1be1dcdb-4c31-5a8c-81de-ac102552ca10", // launch-notes
  "7eb9f239-9393-50b0-bd76-d85eef0511c7", // Jordan Brooks DM
  "d1ec7000-d000-4000-8000-000000000001", // Priya Shah DM
] as const;

export const ANNOUNCEMENT_DEMO_SECTION_STORE = {
  version: 1,
  sections: [
    { id: "the-hive", name: "The Hive", icon: "🐝", order: 0 },
    {
      id: "product-garden",
      name: "Product",
      icon: "🛠️",
      order: 1,
    },
    { id: "launch-swarm", name: "Launch Swarm", icon: "🚀", order: 2 },
  ],
  assignments: {
    "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50": "the-hive",
    "9dae0116-799b-5071-a0a8-fdd30a91a35d": "the-hive",
    "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9": "product-garden",
    "4e91b4d2-8207-4d63-9b1a-3c72a8f01142": "product-garden",
    "b5e2f8a1-3c44-5912-9e67-4a8d1f2b3c4e": "product-garden",
    "94a444a4-c0a3-5966-ab05-530c6ddc2301": "product-garden",
    "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11": "product-garden",
    "c6f3a9b2-4d55-5a23-bf78-5b9e2g3c5d6f": "launch-swarm",
    "1be1dcdb-4c31-5a8c-81de-ac102552ca10": "launch-swarm",
    "3c2d9f0a-1b44-5e77-9a21-6f8b0c4d2e91": "launch-swarm",
  },
} as const;

export const ANNOUNCEMENT_DEMO_PROJECTS = [
  {
    dtag: "flight-path",
    name: "flight-path",
    description:
      "A faster, calmer navigation foundation for Buzz on desktop and mobile.",
    owner: "engineer",
    contributors: ["viewer", "designer", "qa", "data"],
    activityLevel: 4,
  },
  {
    dtag: "nectar",
    name: "nectar",
    description:
      "Customer signals, product research, and insights that shape the roadmap.",
    owner: "viewer",
    contributors: ["designer", "marketing", "researcher", "support"],
    activityLevel: 2,
  },
  {
    dtag: "comb-kit",
    name: "comb-kit",
    description:
      "Buzz's shared design system for coherent, expressive product surfaces.",
    owner: "designer",
    contributors: ["engineer", "producer", "researcher"],
    activityLevel: 3,
  },
  {
    dtag: "swarm-launch",
    name: "swarm-launch",
    description:
      "The announcement campaign, launch assets, and coordinated rollout plan.",
    owner: "marketing",
    contributors: ["viewer", "designer", "producer", "community", "data"],
    activityLevel: 3,
  },
] as const;

export const ANNOUNCEMENT_DEMO_PROJECT_SUBJECTS = [
  "Polish the flight-path transition",
  "Refine the launch story",
  "Add mobile handoff analytics",
  "Review the final motion pass",
  "Tighten contribution summaries",
  "Prepare the announcement build",
  "Update Comb Kit components",
  "Capture the release walkthrough",
] as const;

export const ANNOUNCEMENT_DEMO_MESSAGES: Record<
  string,
  ReadonlyArray<{
    id: string;
    author: AnnouncementDemoPersonKey;
    minutesAgo: number;
    content: string;
    kind?: 9 | 45001;
    extraTags?: ReadonlyArray<ReadonlyArray<string>>;
    reactions?: ReadonlyArray<{
      emoji: string;
      authors: ReadonlyArray<AnnouncementDemoPersonKey>;
      minutesAgo?: number;
    }>;
  }>
> = {
  "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50": [
    {
      id: "announcement-demo-hive-old-1",
      author: "viewer",
      minutesAgo: 4_620,
      content:
        "Quick heads-up: we’re moving the announcement recording to Thursday.",
      reactions: [
        { emoji: "👍", authors: ["marketing", "producer", "engineer"] },
      ],
    },
    {
      id: "announcement-demo-hive-old-2",
      author: "marketing",
      minutesAgo: 4_604,
      content: "Works for the launch calendar.",
    },
    {
      id: "announcement-demo-hive-old-3",
      author: "producer",
      minutesAgo: 4_590,
      content: "I’ll shift the edit review and send a new capture plan.",
    },
    {
      id: "announcement-demo-hive-old-4",
      author: "viewer",
      minutesAgo: 4_574,
      content:
        "Thank you. Same publishing window, just a calmer recording day.",
    },
    {
      id: "announcement-demo-hive-1",
      author: "viewer",
      minutesAgo: 96,
      content: "Morning! The announcement build is ready for a full-team pass.",
    },
    {
      id: "announcement-demo-hive-2",
      author: "viewer",
      minutesAgo: 93,
      content:
        "Please drop launch-only notes in #launch-notes so we keep one source of truth.",
    },
    {
      id: "announcement-demo-hive-3",
      author: "marketing",
      minutesAgo: 71,
      content:
        "Copy is locked on my side. Press kit is still in review, but there are no blockers.",
    },
    {
      id: "announcement-demo-hive-4",
      author: "producer",
      minutesAgo: 68,
      content: "Great. I have the capture plan and a backup shot list ready.",
    },
    {
      id: "announcement-demo-hive-5",
      author: "qa",
      minutesAgo: 46,
      content: "Final smoke pass is clean ✅",
      reactions: [
        { emoji: "🎉", authors: ["viewer", "engineer", "designer"] },
        { emoji: "🙌", authors: ["marketing", "producer"] },
      ],
    },
    {
      id: "announcement-demo-hive-6",
      author: "qa",
      minutesAgo: 45,
      content: "I’m checking notifications one more time, just to be safe.",
    },
    {
      id: "announcement-demo-hive-7",
      author: "viewer",
      minutesAgo: 18,
      content:
        "Thank you, Marcus. And thank you all for making this final stretch feel so calm.",
    },
    {
      id: "announcement-demo-hive-8",
      author: "community",
      minutesAgo: 4,
      content: "Preview invites are queued for Thursday morning.",
      reactions: [{ emoji: "🐝", authors: ["marketing", "viewer"] }],
    },
  ],
  "9dae0116-799b-5071-a0a8-fdd30a91a35d": [
    {
      id: "announcement-demo-waggle-old-1",
      author: "community",
      minutesAgo: 5_920,
      content: "I tried the new channel sections with zero instructions.",
    },
    {
      id: "announcement-demo-waggle-old-2",
      author: "designer",
      minutesAgo: 5_914,
      content: "And?",
    },
    {
      id: "announcement-demo-waggle-old-3",
      author: "community",
      minutesAgo: 5_908,
      content: "Found everything. No notes. Slightly suspicious.",
      reactions: [
        { emoji: "😂", authors: ["designer", "engineer", "researcher"] },
      ],
    },
    {
      id: "announcement-demo-waggle-old-4",
      author: "researcher",
      minutesAgo: 3_130,
      content: "Customer session quote of the day: “Oh, that’s where it went.”",
    },
    {
      id: "announcement-demo-waggle-old-5",
      author: "support",
      minutesAgo: 3_121,
      content: "Honestly, that might be our whole product strategy.",
      reactions: [
        { emoji: "💯", authors: ["viewer", "data"] },
        { emoji: "🐝", authors: ["community"] },
      ],
    },
    {
      id: "announcement-demo-waggle-old-6",
      author: "engineer",
      minutesAgo: 1_755,
      content: "New search animation is on staging.",
    },
    {
      id: "announcement-demo-waggle-old-7",
      author: "qa",
      minutesAgo: 1_748,
      content: "I will pretend I didn’t see that until after lunch.",
    },
    {
      id: "announcement-demo-waggle-1",
      author: "support",
      minutesAgo: 74,
      content: "A preview team called Buzz “surprisingly calm” this morning.",
    },
    {
      id: "announcement-demo-waggle-2",
      author: "designer",
      minutesAgo: 66,
      content: "Oh, I love that.",
    },
    {
      id: "announcement-demo-waggle-3",
      author: "designer",
      minutesAgo: 65,
      content:
        "I softened the selected states yesterday, so hopefully it feels even calmer now.",
    },
    {
      id: "announcement-demo-waggle-4",
      author: "data",
      minutesAgo: 56,
      content:
        "The numbers agree. People are finding active work faster and bouncing around less.",
    },
    {
      id: "announcement-demo-waggle-5",
      author: "community",
      minutesAgo: 48,
      content: "Can we steal “surprisingly calm” for the launch recap?",
    },
    {
      id: "announcement-demo-waggle-6",
      author: "marketing",
      minutesAgo: 46,
      content: "Already wrote it down 😂",
    },
    {
      id: "announcement-demo-waggle-7",
      author: "engineer",
      minutesAgo: 31,
      content: "Release candidate is green on desktop and mobile 🎉",
      reactions: [
        { emoji: "🎉", authors: ["viewer", "designer", "marketing", "qa"] },
      ],
    },
    {
      id: "announcement-demo-waggle-8",
      author: "engineer",
      minutesAgo: 29,
      content: "Please nobody breathe on main for the next hour.",
    },
    {
      id: "announcement-demo-waggle-9",
      author: "qa",
      minutesAgo: 23,
      content: "Too late. I looked at it.",
    },
    {
      id: "announcement-demo-waggle-10",
      author: "qa",
      minutesAgo: 18,
      content: "Still green.",
      reactions: [{ emoji: "😅", authors: ["engineer", "designer"] }],
    },
  ],
  "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9": [
    {
      id: "announcement-demo-flight-old-1",
      author: "engineer",
      minutesAgo: 7_230,
      content: "First transition pass is ready.",
    },
    {
      id: "announcement-demo-flight-old-2",
      author: "engineer",
      minutesAgo: 7_226,
      content: "https://github.com/block/buzz/pull/1768",
      reactions: [{ emoji: "👀", authors: ["qa", "designer"] }],
    },
    {
      id: "announcement-demo-flight-old-3",
      author: "qa",
      minutesAgo: 7_205,
      content: "Pulling it now.",
    },
    {
      id: "announcement-demo-flight-old-4",
      author: "qa",
      minutesAgo: 7_172,
      content: "Found one focus jump when you reverse direction quickly.",
    },
    {
      id: "announcement-demo-flight-old-5",
      author: "engineer",
      minutesAgo: 7_151,
      content: "Good catch. Fixed and pushed.",
      reactions: [{ emoji: "⚡", authors: ["qa"] }],
    },
    {
      id: "announcement-demo-flight-old-6",
      author: "designer",
      minutesAgo: 4_390,
      content: "Motion feels close. Can we shave about 40ms off the settle?",
    },
    {
      id: "announcement-demo-flight-old-7",
      author: "engineer",
      minutesAgo: 4_378,
      content: "Yep. Trying 180ms now.",
    },
    {
      id: "announcement-demo-flight-old-8",
      author: "designer",
      minutesAgo: 4_360,
      content: "That’s it.",
      reactions: [
        { emoji: "✨", authors: ["engineer", "viewer"] },
        { emoji: "👍", authors: ["qa"] },
      ],
    },
    {
      id: "announcement-demo-flight-old-9",
      author: "data",
      minutesAgo: 2_910,
      content: "Navigation sample jumped overnight. This path is getting used.",
    },
    {
      id: "announcement-demo-flight-old-10",
      author: "viewer",
      minutesAgo: 2_895,
      content: "Great. Let’s make it the center of the film sequence.",
    },
    {
      id: "announcement-demo-flight-1",
      author: "data",
      minutesAgo: 69,
      content:
        "Fresh navigation sample is in. Channel → project is now the most common multi-surface flow.",
    },
    {
      id: "announcement-demo-flight-2",
      author: "engineer",
      minutesAgo: 58,
      content: "Nice. The transitions are in.",
    },
    {
      id: "announcement-demo-flight-3",
      author: "engineer",
      minutesAgo: 56,
      content:
        "Kept them quick, but there’s enough movement to understand the context change.",
    },
    {
      id: "announcement-demo-flight-4",
      author: "qa",
      minutesAgo: 47,
      content: "Testing reduced motion now.",
    },
    {
      id: "announcement-demo-flight-5",
      author: "qa",
      minutesAgo: 42,
      content: "Keyboard path is clean too. Focus lands where it should.",
    },
    {
      id: "announcement-demo-flight-6",
      author: "designer",
      minutesAgo: 36,
      content:
        "The speed feels right. I’d pause for half a beat on the project header in the recording.",
    },
    {
      id: "announcement-demo-flight-linear",
      author: "engineer",
      minutesAgo: 33,
      content:
        "I tracked the last timing polish in [BUZ-482 · Capture transition](https://linear.app/honeycomb/issue/BUZ-482/capture-transition-polish).",
      reactions: [{ emoji: "👀", authors: ["qa"] }],
    },
    {
      id: "announcement-demo-flight-7",
      author: "viewer",
      minutesAgo: 28,
      content: "Agreed. That’s where the idea becomes legible.",
    },
    {
      id: "announcement-demo-flight-8",
      author: "producer",
      minutesAgo: 19,
      content:
        "I’ll use one clean cursor move, let the project settle, then type a short update.",
    },
    {
      id: "announcement-demo-flight-9",
      author: "engineer",
      minutesAgo: 13,
      content:
        "Demo build is running. Final cleanup is here: https://github.com/block/buzz/pull/1768",
      reactions: [
        { emoji: "✅", authors: ["qa", "viewer"] },
        { emoji: "🚀", authors: ["producer"] },
      ],
    },
    {
      id: "announcement-demo-flight-10",
      author: "viewer",
      minutesAgo: 7,
      content: "Perfect. That’s the move.",
      reactions: [{ emoji: "🎬", authors: ["producer", "marketing"] }],
    },
  ],
  "4e91b4d2-8207-4d63-9b1a-3c72a8f01142": [
    {
      id: "announcement-demo-engineering-old-1",
      author: "engineer",
      minutesAgo: 6_240,
      content:
        "I moved reconnect ownership into the subscription manager. Less state crossing the bridge now.",
    },
    {
      id: "announcement-demo-engineering-old-2",
      author: "qa",
      minutesAgo: 6_231,
      content: "Nice. I’ll run the sleep / wake matrix against it.",
    },
    {
      id: "announcement-demo-engineering-old-3",
      author: "qa",
      minutesAgo: 6_198,
      content: "First ten loops are clean.",
      reactions: [{ emoji: "✅", authors: ["engineer", "viewer"] }],
    },
    {
      id: "announcement-demo-engineering-old-4",
      author: "engineer",
      minutesAgo: 6_194,
      content: "love to hear it",
    },
    {
      id: "announcement-demo-engineering-old-5",
      author: "data",
      minutesAgo: 4_330,
      content: "Reconnect p95 dropped from 2.8s to 1.1s on the canary cohort.",
      reactions: [{ emoji: "⚡", authors: ["engineer", "qa"] }],
    },
    {
      id: "announcement-demo-engineering-old-6",
      author: "engineer",
      minutesAgo: 4_318,
      content: "That lines up with removing the second backoff window.",
    },
    {
      id: "announcement-demo-engineering-old-7",
      author: "support",
      minutesAgo: 2_915,
      content: "Two preview teams reported a duplicated message after sleep.",
    },
    {
      id: "announcement-demo-engineering-old-8",
      author: "engineer",
      minutesAgo: 2_910,
      content: "Same OS version?",
    },
    {
      id: "announcement-demo-engineering-old-9",
      author: "support",
      minutesAgo: 2_904,
      content: "Both on macOS 15. One Intel, one Apple silicon.",
    },
    {
      id: "announcement-demo-engineering-old-10",
      author: "engineer",
      minutesAgo: 1_860,
      content:
        "Reproduced. The old generation can deliver once after the new one starts.",
    },
    {
      id: "announcement-demo-engineering-pr",
      author: "engineer",
      minutesAgo: 1_842,
      content:
        "Guard is up for review: https://github.com/block/buzz/pull/2037",
      reactions: [{ emoji: "👀", authors: ["qa", "viewer"] }],
    },
    {
      id: "announcement-demo-engineering-old-11",
      author: "qa",
      minutesAgo: 1_831,
      content: "Pulling it now.",
    },
    {
      id: "announcement-demo-engineering-old-12",
      author: "qa",
      minutesAgo: 1_804,
      content: "Twenty reconnects, no duplicates.",
      reactions: [{ emoji: "🙌", authors: ["engineer", "support"] }],
    },
    {
      id: "announcement-demo-engineering-1",
      author: "engineer",
      minutesAgo: 94,
      content: "Tightened the generation guard this morning.",
    },
    {
      id: "announcement-demo-engineering-2",
      author: "engineer",
      minutesAgo: 92,
      content: "```ts\nif (generation !== activeGeneration) return;\n```",
    },
    {
      id: "announcement-demo-engineering-3",
      author: "qa",
      minutesAgo: 76,
      content: "Manual reconnect and offline recovery both look good.",
    },
    {
      id: "announcement-demo-engineering-4",
      author: "data",
      minutesAgo: 61,
      content:
        "Telemetry is clean too — no doubled fan-out in the latest sample.",
    },
    {
      id: "announcement-demo-engineering-linear",
      author: "engineer",
      minutesAgo: 43,
      content:
        "I put the remaining soak checklist in [BUZ-519 · Reconnect reliability](https://linear.app/honeycomb/issue/BUZ-519/reconnect-reliability).",
      reactions: [{ emoji: "✅", authors: ["qa"] }],
    },
    {
      id: "announcement-demo-engineering-5",
      author: "support",
      minutesAgo: 22,
      content: "Customer replay passed on both affected machines.",
    },
    {
      id: "announcement-demo-engineering-6",
      author: "engineer",
      minutesAgo: 5,
      content: "Release build is green. I’m doing one last sleep / wake pass.",
      reactions: [
        { emoji: "🚀", authors: ["qa", "viewer"] },
        { emoji: "🙏", authors: ["support"] },
      ],
    },
  ],
  "b5e2f8a1-3c44-5912-9e67-4a8d1f2b3c4e": [
    {
      id: "announcement-demo-design-old-1",
      author: "designer",
      minutesAgo: 6_080,
      content: "Trying a quieter selected state for the sidebar.",
    },
    {
      id: "announcement-demo-design-old-2",
      author: "designer",
      minutesAgo: 6_074,
      content:
        "Before / after is in the PR: https://github.com/block/buzz/pull/1712",
      reactions: [{ emoji: "👀", authors: ["researcher", "producer"] }],
    },
    {
      id: "announcement-demo-design-old-3",
      author: "researcher",
      minutesAgo: 6_050,
      content: "The quieter version wins for me.",
    },
    {
      id: "announcement-demo-design-old-4",
      author: "producer",
      minutesAgo: 6_041,
      content: "Same. It reads better in motion.",
      reactions: [{ emoji: "👍", authors: ["designer", "viewer"] }],
    },
    {
      id: "announcement-demo-design-old-5",
      author: "designer",
      minutesAgo: 3_270,
      content: "New avatar crops are in.",
    },
    {
      id: "announcement-demo-design-old-6",
      author: "producer",
      minutesAgo: 3_260,
      content: "These feel much more like a real team.",
    },
    {
      id: "announcement-demo-design-old-7",
      author: "designer",
      minutesAgo: 3_253,
      content: "Exactly what I was hoping for 🙌",
      reactions: [{ emoji: "❤️", authors: ["producer", "researcher"] }],
    },
    {
      id: "announcement-demo-design-1",
      author: "researcher",
      minutesAgo: 78,
      content:
        "People understood the sections immediately in the latest sessions.",
    },
    {
      id: "announcement-demo-design-2",
      author: "researcher",
      minutesAgo: 76,
      content:
        "The “Product” section was the favorite. Organized, but not too formal.",
    },
    {
      id: "announcement-demo-design-3",
      author: "designer",
      minutesAgo: 64,
      content:
        "That’s helpful. I’ve made the hierarchy a little clearer and quieted the channel icons.",
    },
    {
      id: "announcement-demo-design-4",
      author: "designer",
      minutesAgo: 62,
      content: "The active state holds up in both themes now.",
    },
    {
      id: "announcement-demo-design-5",
      author: "producer",
      minutesAgo: 51,
      content: "Looks great on camera.",
      reactions: [{ emoji: "✨", authors: ["designer", "viewer"] }],
    },
    {
      id: "announcement-demo-design-6",
      author: "producer",
      minutesAgo: 49,
      content:
        "Can we keep the cursor away from the avatars in the opening shot?",
    },
    {
      id: "announcement-demo-design-7",
      author: "designer",
      minutesAgo: 43,
      content: "Yep. I’ll frame the movement around the channel names.",
    },
    {
      id: "announcement-demo-design-8",
      author: "engineer",
      minutesAgo: 34,
      content: "Motion pass is smooth at the recording size.",
    },
    {
      id: "announcement-demo-design-9",
      author: "viewer",
      minutesAgo: 22,
      content: "Let’s keep the opening wide. The workspace should land first.",
    },
    {
      id: "announcement-demo-design-10",
      author: "producer",
      minutesAgo: 12,
      content:
        "Works for me. I’m calling this sequence locked.\n\n![image](/demo/attachments/flight-path-capture.png)",
      extraTags: [
        [
          "imeta",
          "url /demo/attachments/flight-path-capture.png",
          "m image/png",
          "x f0b8ade18d5dfc5a7780736d8ce495d9b946d4bef831a21da55806f44a6dafd1",
          "size 266615",
          "dim 1200x717",
          "filename flight-path-capture.png",
        ],
      ],
      reactions: [
        { emoji: "🔥", authors: ["designer", "marketing"] },
        { emoji: "🎬", authors: ["viewer"] },
      ],
    },
    {
      id: "announcement-demo-design-doc",
      author: "designer",
      minutesAgo: 10,
      content:
        "I added the final frames to the [Announcement storyboard](https://docs.google.com/document/d/1BUZZANNOUNCEMENTSTORYBOARD/edit).",
    },
    {
      id: "announcement-demo-design-doc-reply",
      author: "producer",
      minutesAgo: 8,
      content: "Perfect, I see them.",
      reactions: [{ emoji: "🙏", authors: ["designer"] }],
    },
  ],
  "94a444a4-c0a3-5966-ab05-530c6ddc2301": [
    {
      id: "announcement-demo-mobile-old-1",
      author: "engineer",
      minutesAgo: 5_030,
      content: "Draft handoff prototype is on staging.",
    },
    {
      id: "announcement-demo-mobile-old-2",
      author: "support",
      minutesAgo: 5_011,
      content: "Trying it with the support workspace now.",
    },
    {
      id: "announcement-demo-mobile-old-3",
      author: "support",
      minutesAgo: 4_998,
      content: "This is good. I forgot which device I started on.",
      reactions: [
        { emoji: "🎯", authors: ["engineer", "designer"] },
        { emoji: "🙌", authors: ["viewer"] },
      ],
    },
    {
      id: "announcement-demo-mobile-old-4",
      author: "qa",
      minutesAgo: 3_615,
      content: "Found an edge case with an old notification replacing a draft.",
    },
    {
      id: "announcement-demo-mobile-old-5",
      author: "engineer",
      minutesAgo: 3_590,
      content: "Reproduced. Fixing now.",
    },
    {
      id: "announcement-demo-mobile-old-6",
      author: "engineer",
      minutesAgo: 3_552,
      content: "Fix is up: https://github.com/block/buzz/pull/1674",
      reactions: [{ emoji: "✅", authors: ["qa", "support"] }],
    },
    {
      id: "announcement-demo-mobile-old-7",
      author: "qa",
      minutesAgo: 3_531,
      content: "Confirmed. Draft wins now.",
    },
    {
      id: "announcement-demo-mobile-1",
      author: "support",
      minutesAgo: 91,
      content:
        "Most common mobile question: will opening a thread mess with where I left off on desktop?",
    },
    {
      id: "announcement-demo-mobile-2",
      author: "engineer",
      minutesAgo: 78,
      content:
        "The draft follows you now. https://github.com/block/buzz/pull/1674",
      reactions: [{ emoji: "🙌", authors: ["support", "designer"] }],
    },
    {
      id: "announcement-demo-mobile-3",
      author: "engineer",
      minutesAgo: 76,
      content:
        "Active channel and thread context too. It should feel like the same session.",
    },
    {
      id: "announcement-demo-mobile-4",
      author: "designer",
      minutesAgo: 67,
      content:
        "Nice. I’ll make sure the recording shows the same draft on both screens.",
    },
    {
      id: "announcement-demo-mobile-5",
      author: "data",
      minutesAgo: 56,
      content:
        "Handoff completion is up in the preview group, especially from notifications.",
    },
    {
      id: "announcement-demo-mobile-6",
      author: "qa",
      minutesAgo: 45,
      content: "Tested cold launch, background resume, and expired sessions.",
    },
    {
      id: "announcement-demo-mobile-7",
      author: "qa",
      minutesAgo: 43,
      content: "All clean.",
      reactions: [{ emoji: "✅", authors: ["engineer", "support"] }],
    },
    {
      id: "announcement-demo-mobile-8",
      author: "support",
      minutesAgo: 35,
      content:
        "The recovery copy feels much better too. Clear, but not alarming.",
    },
    {
      id: "announcement-demo-mobile-9",
      author: "designer",
      minutesAgo: 26,
      content: "Clean handoff recording is in the folder.",
      reactions: [{ emoji: "🎥", authors: ["producer"] }],
    },
  ],
  "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11": [
    {
      id: "announcement-demo-garden-old-1",
      author: "researcher",
      minutesAgo: 7_410,
      kind: 45001,
      content:
        "Interview notes: where new teammates look first when they enter an established workspace.",
    },
    {
      id: "announcement-demo-garden-old-2",
      author: "support",
      minutesAgo: 5_870,
      kind: 45001,
      content:
        "Idea: a gentle “what changed while you were away?” summary for busy channels.",
    },
    {
      id: "announcement-demo-garden-old-3",
      author: "researcher",
      minutesAgo: 3_260,
      kind: 45001,
      content:
        "Research plan: test real workspace context against a generic onboarding checklist.",
    },
    {
      id: "announcement-demo-garden-1",
      author: "researcher",
      minutesAgo: 128,
      kind: 45001,
      content:
        "Research synthesis: what should a calm first five minutes in Buzz feel like?",
    },
    {
      id: "announcement-demo-garden-2",
      author: "researcher",
      minutesAgo: 111,
      kind: 45001,
      content:
        "Onboarding idea: use the team’s real channels and current work instead of a generic checklist.",
    },
    {
      id: "announcement-demo-garden-3",
      author: "support",
      minutesAgo: 93,
      kind: 45001,
      content:
        "Could repeated support answers become lightweight guidance inside the relevant channel?",
    },
    {
      id: "announcement-demo-garden-4",
      author: "data",
      minutesAgo: 67,
      kind: 45001,
      content:
        "Signal: teams with a few thoughtfully named sections return to active work faster.",
    },
    {
      id: "announcement-demo-garden-5",
      author: "community",
      minutesAgo: 34,
      kind: 45001,
      content:
        "Community idea: a small gallery of real workspace patterns after launch.",
    },
  ],
  "c6f3a9b2-4d55-5a23-bf78-5b9e2g3c5d6f": [
    {
      id: "announcement-demo-pollen-old-1",
      author: "marketing",
      minutesAgo: 6_350,
      content: "First launch storyboard is ready for comments.",
    },
    {
      id: "announcement-demo-pollen-old-2",
      author: "community",
      minutesAgo: 6_322,
      content:
        "The community beat is strong. I’d bring it ten seconds earlier.",
    },
    {
      id: "announcement-demo-pollen-old-3",
      author: "marketing",
      minutesAgo: 6_309,
      content: "Good call. Moving it before the mobile handoff.",
      reactions: [{ emoji: "👍", authors: ["community", "producer"] }],
    },
    {
      id: "announcement-demo-pollen-old-4",
      author: "producer",
      minutesAgo: 4_880,
      content: "Rough cut is exporting. No color pass yet.",
    },
    {
      id: "announcement-demo-pollen-old-5",
      author: "marketing",
      minutesAgo: 4_842,
      content: "The pacing works. The middle title can be shorter.",
    },
    {
      id: "announcement-demo-pollen-old-6",
      author: "producer",
      minutesAgo: 4_831,
      content: "Agreed. Cutting it to two words.",
      reactions: [{ emoji: "✂️", authors: ["marketing"] }],
    },
    {
      id: "announcement-demo-pollen-old-7",
      author: "community",
      minutesAgo: 2_220,
      content:
        "Preview group asked if they can share behind-the-scenes stills.",
    },
    {
      id: "announcement-demo-pollen-old-8",
      author: "marketing",
      minutesAgo: 2_204,
      content: "Yes, after the main post is live.",
    },
    {
      id: "announcement-demo-pollen-1",
      author: "community",
      minutesAgo: 87,
      content:
        "Announcement-day community flow is mapped, including the live Q&A.",
    },
    {
      id: "announcement-demo-pollen-2",
      author: "marketing",
      minutesAgo: 73,
      content:
        "Perfect. The campaign is still three beats: see the team, follow the work, keep moving.",
    },
    {
      id: "announcement-demo-pollen-3",
      author: "marketing",
      minutesAgo: 71,
      content: "Let’s keep every caption anchored to one of those.",
    },
    {
      id: "announcement-demo-pollen-4",
      author: "producer",
      minutesAgo: 57,
      content: "The edit already maps cleanly to that arc.",
    },
    {
      id: "announcement-demo-pollen-5",
      author: "producer",
      minutesAgo: 55,
      content: "Team in channels, work in projects, then the mobile handoff.",
    },
    {
      id: "announcement-demo-pollen-6",
      author: "community",
      minutesAgo: 42,
      content:
        "I like it. Preview group is ready to share their favorite moment.",
    },
    {
      id: "announcement-demo-pollen-7",
      author: "marketing",
      minutesAgo: 34,
      content: "Their own words, please. No copy-paste script.",
    },
    {
      id: "announcement-demo-pollen-8",
      author: "community",
      minutesAgo: 31,
      content: "Definitely. That was the plan.",
    },
    {
      id: "announcement-demo-pollen-9",
      author: "designer",
      minutesAgo: 18,
      content:
        "Square, vertical, and wide exports are ready.\n\n[launch-social-crops.zip](https://mock.relay/media/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.zip)",
      extraTags: [
        [
          "imeta",
          "url https://mock.relay/media/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.zip",
          "m application/zip",
          "x bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
          "size 18432000",
          "filename launch-social-crops.zip",
        ],
      ],
      reactions: [{ emoji: "🙌", authors: ["marketing", "community"] }],
    },
    {
      id: "announcement-demo-pollen-10",
      author: "marketing",
      minutesAgo: 9,
      content:
        "Found them. These look great. I updated the [Launch content calendar](https://docs.google.com/spreadsheets/d/1BUZZLAUNCHCALENDAR/edit#gid=0).",
      reactions: [{ emoji: "✨", authors: ["designer"] }],
    },
  ],
  "1be1dcdb-4c31-5a8c-81de-ac102552ca10": [
    {
      id: "announcement-demo-launch-notes-old-1",
      author: "marketing",
      minutesAgo: 7_120,
      kind: 45001,
      content: "Launch narrative v1: the three ideas the film needs to prove.",
    },
    {
      id: "announcement-demo-launch-notes-old-2",
      author: "producer",
      minutesAgo: 5_540,
      kind: 45001,
      content: "Edit review notes: pacing, supers, and the mobile transition.",
    },
    {
      id: "announcement-demo-launch-notes-old-3",
      author: "qa",
      minutesAgo: 3_980,
      kind: 45001,
      content:
        "Capture readiness: paths that must remain interactive on camera.",
    },
    {
      id: "announcement-demo-launch-notes-1",
      author: "producer",
      minutesAgo: 137,
      kind: 45001,
      content:
        "Final film review: opening hold, live message send, and mobile handoff.",
    },
    {
      id: "announcement-demo-launch-notes-2",
      author: "marketing",
      minutesAgo: 112,
      kind: 45001,
      content: "Copy lock: headline, product description, and social language.",
    },
    {
      id: "announcement-demo-launch-notes-3",
      author: "qa",
      minutesAgo: 86,
      kind: 45001,
      content: "Recording build checklist and final smoke results.",
    },
    {
      id: "announcement-demo-launch-notes-4",
      author: "producer",
      minutesAgo: 54,
      kind: 45001,
      content:
        "Shot list: clean workspace overview plus backup angles.\n\n[announcement-shot-list.pdf](https://mock.relay/media/cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc.pdf)",
      extraTags: [
        [
          "imeta",
          "url https://mock.relay/media/cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc.pdf",
          "m application/pdf",
          "x cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
          "size 842731",
          "filename announcement-shot-list.pdf",
        ],
      ],
    },
    {
      id: "announcement-demo-launch-notes-5",
      author: "viewer",
      minutesAgo: 22,
      kind: 45001,
      content: "Go/no-go: final capture and publishing checklist.",
    },
  ],
  "3c2d9f0a-1b44-5e77-9a21-6f8b0c4d2e91": [
    {
      id: "announcement-demo-queen-old-1",
      author: "producer",
      minutesAgo: 3_480,
      content: "Private room is open. I’ll keep final-cut notes here.",
    },
    {
      id: "announcement-demo-queen-old-2",
      author: "marketing",
      minutesAgo: 3_462,
      content: "Perfect. I’ll keep publishing changes here too.",
    },
    {
      id: "announcement-demo-queen-old-3",
      author: "viewer",
      minutesAgo: 3_440,
      content: "Small group, fast decisions. Exactly what we need.",
      reactions: [{ emoji: "🐝", authors: ["producer", "marketing"] }],
    },
    {
      id: "announcement-demo-queen-old-4",
      author: "engineer",
      minutesAgo: 1_910,
      content: "Recording build is pinned. I won’t touch the data after today.",
    },
    {
      id: "announcement-demo-queen-old-5",
      author: "qa",
      minutesAgo: 1_892,
      content: "I’ll do the final sweep against that exact build.",
      reactions: [{ emoji: "✅", authors: ["engineer", "viewer"] }],
    },
    {
      id: "announcement-demo-queen-1",
      author: "producer",
      minutesAgo: 66,
      content: "Cut twelve is up. New opening, cleaner mobile transition.",
    },
    {
      id: "announcement-demo-queen-2",
      author: "producer",
      minutesAgo: 64,
      content: "Runtime is 1:12 now.",
    },
    {
      id: "announcement-demo-queen-3",
      author: "viewer",
      minutesAgo: 55,
      content: "Watching now.",
    },
    {
      id: "announcement-demo-queen-4",
      author: "viewer",
      minutesAgo: 51,
      content: "The opening breathes much better.",
      reactions: [{ emoji: "❤️", authors: ["producer", "marketing"] }],
    },
    {
      id: "announcement-demo-queen-5",
      author: "qa",
      minutesAgo: 43,
      content:
        "Every visible interaction is reproducible in the recording build.",
    },
    {
      id: "announcement-demo-queen-6",
      author: "engineer",
      minutesAgo: 32,
      content: "I froze the demo data and added a quick reset.",
    },
    {
      id: "announcement-demo-queen-7",
      author: "engineer",
      minutesAgo: 30,
      content:
        "So we can rehearse without worrying about breaking the starting state.",
    },
    {
      id: "announcement-demo-queen-8",
      author: "producer",
      minutesAgo: 20,
      content: "Sound mix is approved.",
      reactions: [
        { emoji: "🎧", authors: ["marketing"] },
        { emoji: "✅", authors: ["viewer", "qa"] },
      ],
    },
    {
      id: "announcement-demo-queen-9",
      author: "marketing",
      minutesAgo: 8,
      content: "Press, social, and the product page are staged.",
    },
    {
      id: "announcement-demo-queen-10",
      author: "marketing",
      minutesAgo: 3,
      content: "Nothing publishes until we say go.",
      reactions: [{ emoji: "👀", authors: ["viewer", "producer"] }],
    },
  ],
  "f48efb06-0c93-5025-aac9-2e646bb6bfa8": [
    {
      id: "announcement-demo-dm-maya",
      author: "engineer",
      minutesAgo: 11,
      content: "I left the clean build running for your capture session.",
    },
  ],
  "7eb9f239-9393-50b0-bd76-d85eef0511c7": [
    {
      id: "announcement-demo-dm-jordan",
      author: "designer",
      minutesAgo: 16,
      content: "Sending the updated storyboard now — the pacing feels right.",
    },
  ],
  "d1ec7000-d000-4000-8000-000000000001": [
    {
      id: "announcement-demo-dm-priya",
      author: "marketing",
      minutesAgo: 6,
      content:
        "The launch calendar is clear. We can publish whenever the cut lands.",
    },
  ],
};
