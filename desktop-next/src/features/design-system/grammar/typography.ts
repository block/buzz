import type { GrammarSection } from "./types";

export const typographyGrammar: GrammarSection[] = [
  {
    id: "type-selection",
    title: "Choosing typography",
    sources: ["docs/type.roles.md", "docs/type.resolution.draft.json"],
    rows: [
      [
        "Content first",
        "Choose Body for prose, Label for names, Link for navigation, Numeral for amounts, and Display for identity.",
        "Equal size does not make roles interchangeable.",
      ],
      [
        "Weight",
        "Regular supports reading; Medium identifies controls, structure, and numeric values.",
        "Weight belongs to the role; it is not an independent emphasis dial.",
      ],
      [
        "Complete setting",
        "Resolve size, leading, tracking, weight, and family together.",
        "Avoid assembling an unregistered combination.",
      ],
      [
        "Numbers",
        "General numeral roles use Medium, tabular figures, and zero tracking.",
        "Applet anchors have a more specific rule: regular headline roles, described below.",
      ],
      [
        "Links",
        "Use the Link roles and their underline treatment.",
        "A button is an action component; a link is navigation.",
      ],
      [
        "Monospace",
        "BlockUI reserves Cash Sans Mono for its smallest Detail tier.",
        "Buzz uses open-source fonts and has a separate code role; this reference does not change it.",
      ],
      [
        "Collisions",
        "Use page-title for screen identity and numeral-small for an amount, even at the same size.",
        "Use the role’s exclusion rule to resolve ambiguity.",
      ],
    ],
  },
  {
    id: "type-roles",
    title: "The 19 typography roles",
    sources: ["docs/type.roles.md"],
    rows: [
      [
        "display/hero",
        "Expressive welcome or brand statement.",
        "Use page-title for ordinary screen identity; numeral-large for a number.",
      ],
      [
        "display/numeral-large",
        "The main amount the screen concerns.",
        "Use hero for words; numeral-small for a supporting amount.",
      ],
      [
        "display/headline-large",
        "Leading editorial or marketing headline.",
        "Use page-title for application structure.",
      ],
      [
        "display/page-title",
        "Screen identity or modal title.",
        "Use section-title for a content group; headline-small for editorial tone.",
      ],
      [
        "display/headline-small",
        "Secondary editorial headline.",
        "Use page-title for a structural screen title.",
      ],
      [
        "display/numeral-small",
        "A supporting metric, card amount, or row total.",
        "Use a title role when the content names a screen.",
      ],
      [
        "display/section-title",
        "Heading that groups related content.",
        "Use page-title for the whole screen.",
      ],
      [
        "body/body-large",
        "A lead paragraph or blockquote.",
        "Use body-medium for ordinary prose; label-large for a name.",
      ],
      [
        "body/label-large",
        "A prominent control or group label.",
        "Use body-large for prose; label-medium for standard labels.",
      ],
      [
        "body/body-medium",
        "Default paragraphs, primary content, and messages.",
        "Use label-medium for controls; body-small for supporting text.",
      ],
      [
        "body/label-medium",
        "Form labels, control names, and primary row labels.",
        "Use body-medium for running prose.",
      ],
      [
        "body/link-medium",
        "A link at the normal reading scale.",
        "Use Button for a full action control.",
      ],
      [
        "body/body-small",
        "Supporting copy and dense table cells.",
        "Use body-medium for the main reading content.",
      ],
      [
        "body/label-small",
        "Badge text, chip labels, and table headers.",
        "Use body-small for reading text.",
      ],
      [
        "body/link-small",
        "A link in a dense interface.",
        "Use a larger Link role when compactness is unnecessary.",
      ],
      [
        "detail/caption",
        "Metadata, timestamps, help, errors, and footnotes.",
        "Use Body for primary content.",
      ],
      [
        "detail/body-xsmall",
        "Fine-print identifiers and monospace data.",
        "Use caption for prose.",
      ],
      [
        "detail/label-xsmall",
        "Very small data labels and chart annotations.",
        "Use label-small for ordinary compact labels.",
      ],
      [
        "detail/link-xsmall",
        "A small link inside monospace data.",
        "Use a Body Link role outside micro-data contexts.",
      ],
    ],
  },
];
