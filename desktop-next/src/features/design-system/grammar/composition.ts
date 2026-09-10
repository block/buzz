import type { GrammarSection } from "./types";

export const compositionGrammar: GrammarSection[] = [
  {
    id: "intent",
    title: "Intent, archetype, and mode",
    sources: [
      "docs/intent-resolution.spec.md",
      "docs/archetypes-and-modes.spec.md",
    ],
    rows: [
      [
        "Pipeline",
        "Domain → archetype → mode → anchor → visualization → support → density.",
        "Data shape chooses the visualization after the archetype is selected.",
      ],
      [
        "Track",
        "Monitor a changing value.",
        "A goal, event review, or derived recommendation has a different job.",
      ],
      [
        "Goal",
        "Show progress toward a target or limit.",
        "A timeline or member avatar does not turn a goal into Activity.",
      ],
      [
        "Activity",
        "Review events, dates, people, or recent items.",
        "Use the user’s job, not the appearance of the chart, to classify it.",
      ],
      [
        "Insight",
        "Explain a derived finding or suggestion.",
        "Avoid relabeling an ordinary raw metric as an insight.",
      ],
      [
        "Archetype tie-break",
        "The user’s verb overrides the domain default; for vague intent, try Insight → Goal → Activity → Track.",
        "Track is the final fallback, accompanied by a warning.",
      ],
      [
        "Status",
        "The answer leads with current state.",
        "A supporting button or toggle does not make it Action mode.",
      ],
      [
        "Plan",
        "The answer leads with a future date, schedule, or forecast.",
        "Use Action when a recommendation or decision leads the answer.",
      ],
      [
        "Action",
        "The anchor leads with the recommendation; the decision control belongs in support.",
        "This test checks the resolved output; intent-side signal extraction remains proposed.",
      ],
      [
        "Unknown domain",
        "Use neutral tokens and resolve by the user’s job.",
        "Do not invent product tokens or a new taxonomy member.",
      ],
    ],
  },
  {
    id: "slots",
    title: "Applet slots and compatibility",
    sources: [
      "docs/slot-rules.spec.md",
      "docs/intent-resolution.spec.md",
      "docs/anti-slop.spec.md",
    ],
    rows: [
      [
        "Reading order",
        "Heading → A: anchor → B: visualization → C: support.",
        "One subject, one lead answer, at most one visualization, and one support area; no nested applets.",
      ],
      [
        "Heading",
        "A short subject identifier.",
        "Applet kickers use the source’s small mono treatment; desktop artifact headings have different rules.",
      ],
      [
        "amount",
        "A single figure that answers the question.",
        "Applet bare amounts use regular headline-large, overriding the general numeral rule.",
      ],
      [
        "amountWithTrend",
        "A value whose change is part of the answer.",
        "Requires a comparison value; otherwise fall back to amount.",
      ],
      [
        "multiAmount",
        "Two figures that need each other for meaning.",
        "Use amount when only one figure exists.",
      ],
      [
        "text",
        "A named period, subject, or conclusion.",
        "It can accompany a chart when both concern the same topic.",
      ],
      [
        "Visualization",
        "Develop the anchor’s subject with one data-appropriate figure.",
        "A composite can count as one figure when it has one clear primary element.",
      ],
      [
        "Support",
        "A useful takeaway or one available next step.",
        "Actions belong only here; missing actions become context text, not dead buttons.",
      ],
      [
        "Actions",
        "One secondary CTA; two only for an established binary choice.",
        "The applet itself is the primary action. Size permission does not prove renderer readiness.",
      ],
      [
        "Shell",
        "The applet reference specifies radius 32 and padding 24 at each size.",
        "These are applet-specific reference values, not overrides of Buzz density tokens.",
      ],
    ],
  },
  {
    id: "visualization",
    title: "Data shape → visualization",
    sources: [
      "docs/intent-resolution.spec.md",
      "docs/archetypes-and-modes.spec.md",
    ],
    rows: [
      [
        "Per-period ranges",
        "rangeBar",
        "Distinguish volatility from a simple trend.",
      ],
      [
        "Trend over time",
        "line",
        "Do not use a smooth trend to imply unavailable ranges.",
      ],
      [
        "Future projection",
        "forecast",
        "Keep projected and observed values distinguishable.",
      ],
      [
        "Progress to one target",
        "progressRing or progressBar",
        "A collection of goals may need an activityList.",
      ],
      [
        "Parts of a whole",
        "barBreakdown",
        "Keep the focal category separate from neutral context.",
      ],
      ["Dated events", "calendar", "Ordered steps instead call for timeline."],
      [
        "Steps or milestones",
        "timeline",
        "Must survive the available height, or simplify.",
      ],
      [
        "Recent items",
        "activityList",
        "Avoid cramming a multi-row list into a short band.",
      ],
      [
        "People",
        "avatarGroup",
        "Membership can support a primary progress visualization.",
      ],
      [
        "A specific card",
        "cardAsset",
        "Use an existing taxonomy member and verified asset.",
      ],
      [
        "Locations",
        "map",
        "The reference marks glossary availability as pending; verify renderer support.",
      ],
    ],
  },
  {
    id: "palettes",
    title: "Archetype palettes",
    sources: ["docs/archetypes-and-modes.spec.md"],
    rows: [
      [
        "Track",
        "Range/line series, breakdown, item list, or metric.",
        "Amounts lead; support can be context, one action, attribution, or a reversible toggle.",
      ],
      [
        "Goal",
        "Progress, forecast, goals, payoff timeline, shared participants, or line.",
        "Amounts lead; support is context or one action.",
      ],
      [
        "Activity",
        "Calendar, item list, timeline, participants, or line.",
        "An amount or text leads; support is context, one action, or a labeled push.",
      ],
      [
        "Insight",
        "Breakdown, line, forecast, progress, or map.",
        "A conclusion or amount leads; support is context, one action, or toggle.",
      ],
      [
        "Validation",
        "Check the selected visualization against the archetype’s palette.",
        "Do not change archetype merely to justify an out-of-palette visualization.",
      ],
    ],
  },
  {
    id: "mode-matrix",
    title: "The 12 default compositions",
    sources: ["docs/archetypes-and-modes.spec.md"],
    rows: [
      [
        "Track · Status",
        "Amount + trend → range/line → context or toggle.",
        "Defaults describe Large applets; apply size rules afterward.",
      ],
      [
        "Track · Plan",
        "Amount → forecast → schedule context.",
        "Keep the future framing explicit.",
      ],
      [
        "Track · Action",
        "Recommendation → metric/line → action.",
        "Recommendation leads; the metric becomes evidence.",
      ],
      [
        "Goal · Status",
        "Amount → progress or multiple goals → action.",
        "One target and many targets use different visualizations.",
      ],
      [
        "Goal · Plan",
        "Amount → forecast → context.",
        "The goal persists while framing becomes forward-looking.",
      ],
      [
        "Goal · Action",
        "Recommendation → metric → action.",
        "One clear next step.",
      ],
      [
        "Activity · Status",
        "Amount → recent items → context.",
        "Report current or recent state.",
      ],
      [
        "Activity · Plan",
        "Period label → calendar → context.",
        "The slot rules retain a support line even where older matrix shorthand omits it.",
      ],
      [
        "Activity · Action",
        "Recommendation → participants/metric → action.",
        "Two controls require a meaningful binary decision.",
      ],
      [
        "Insight · Status",
        "Amount → line/breakdown → context.",
        "Keep one synthesized subject.",
      ],
      [
        "Insight · Plan",
        "Amount → line → attributed context.",
        "Make the future scope clear.",
      ],
      [
        "Insight · Action",
        "Recommendation → breakdown → action.",
        "The evidence must support the recommendation.",
      ],
    ],
  },
  {
    id: "degradation",
    title: "Size and missing-data fallbacks",
    sources: ["docs/slot-rules.spec.md"],
    rows: [
      [
        "Large → Medium",
        "Reduce height at full width, remove secondary anchor detail, and compact the visualization.",
        "Keep the main answer and support; action controls remain valid at Medium.",
      ],
      [
        "Medium → Small",
        "Reduce width, step the anchor down, collapse paired amounts, and remove secondary labels.",
        "This is content adaptation, not uniform scaling or Buzz’s Normal/Compact switch.",
      ],
      [
        "Small support",
        "Replace the action control with a status takeaway.",
        "Only the source-verified compact +/− pattern is an interactive exception; do not generalize it.",
      ],
      [
        "Visualization fit",
        "Compact a single band or object before dropping it.",
        "A multi-row timeline or list may fail to fit; the Medium band can be shorter than Small.",
      ],
      [
        "Minimum",
        "Keep the subject heading, primary answer, and a line of support.",
        "An Action-mode text answer may merge into support at Medium without losing its meaning.",
      ],
      [
        "Missing comparison",
        "Replace amountWithTrend with amount.",
        "Never fabricate movement or comparison data.",
      ],
      [
        "Missing figure",
        "Use a text takeaway.",
        "An empty numeric placeholder is not an answer.",
      ],
      [
        "Sparse or absent series",
        "Try a metric or short list; omit the visualization when no meaningful data remains.",
        "Collapse its space instead of rendering an empty slot.",
      ],
      [
        "Missing action",
        "Show useful context.",
        "Never offer an unavailable operation as a working button.",
      ],
    ],
  },
  {
    id: "generation",
    title: "Generated responses and consequence",
    sources: ["Design.md", "docs/role-join.md", "docs/anti-slop.spec.md"],
    rows: [
      [
        "Response grammar",
        "Answer → evidence → meaning → actions.",
        "This response recipe is related to, but not identical to, an applet’s Heading/A/B/C slots.",
      ],
      [
        "Structure versus values",
        "Generated content selects supported structure; the renderer supplies design values.",
        "The response contract is not a channel for arbitrary CSS or new token values.",
      ],
      [
        "Specificity",
        "Lead with one concrete answer, evidence on that subject, and a useful interpretation.",
        "Avoid co-equal metric grids, filler text, and interpretations that only repeat the answer.",
      ],
      [
        "Traceability",
        "Tie component and slot choices to a documented rule or reference example.",
        "A visually plausible invention is not proof of supported composition.",
      ],
      [
        "Artifact",
        "A persistent work object with identity, context, lifecycle, and ordered content blocks.",
        "A hero is conditional; a block can be bare or card-backed. An applet is not synonymous with every card.",
      ],
      [
        "R0 / R1 / R2",
        "Informational → personalized read → recommendation; add source evidence and reasoning as consequence grows.",
        "The source’s applet corpus supports these levels; this is a reference policy.",
      ],
      [
        "R3 / R4 / R5",
        "Prepared draft → reversible execution → sensitive execution; require preview, confirmation, receipt, and appropriate review/audit.",
        "The source marks these as forward-looking and blocks R3+ generation pending a reviewed baseline.",
      ],
      [
        "Authorization",
        "Show → explain → suggest → draft → preview → confirm → execute; escalate when needed.",
        "Do not promote intent into authorization or show success before backend confirmation.",
      ],
      [
        "Readiness",
        "Check target support, required states, provenance, and known gaps before generation.",
        "A component being legal in the size grammar does not mean it is approved or implemented.",
      ],
    ],
  },
  {
    id: "buzz-boundary",
    title: "Reference status and Buzz differences",
    sources: [
      "docs/type.resolution.draft.json",
      "docs/color.roles.md",
      "docs/role-join.md",
      "docs/archetypes-and-modes.spec.md",
      "foundations/typography/typography-foundation.md",
    ],
    rows: [
      [
        "Reference snapshot",
        "Summarized from the linked BlockUI sources, verified September 10, 2026.",
        "Drafts, proposed rules, and unresolved platform differences retain their source status.",
      ],
      [
        "Buzz fonts and type",
        "Buzz uses Inter and JetBrains Mono with its existing ten-role, rem-based scale.",
        "This page documents BlockUI’s nineteen roles; it does not silently replace the Buzz type contract.",
      ],
      [
        "Buzz conventions",
        "Buzz retains sentence-case labels, semantic geometry, and shared Normal/Compact density.",
        "BlockUI’s mono uppercase applet kicker, applet sizes, and specific numeric settings are reference-specific.",
      ],
      [
        "Usage coverage",
        "The upstream role join distinguishes exact authored rules, family matches, and gaps.",
        "Do not assume every available token has complete usage guidance.",
      ],
      [
        "Open questions",
        "Source gaps include intent-side signal extraction, role pairing, responsive type breakpoints, and some platform bindings.",
        "This outline does not resolve those gaps or implement the BlockUI planner in Buzz.",
      ],
    ],
  },
];
