use serde::{Deserialize, Serialize};

/// Shivai's normalized light/dark presentation contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewPresentationVariants {
    pub format_version: u8,
    pub dark: WorldViewPresentationModel,
    pub light: WorldViewPresentationModel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewPresentationModel {
    pub graph: WorldViewGraphModel,
    pub revision: Option<String>,
    pub selection: WorldViewSelection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewSelection {
    pub realm_qualified_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_preference_qualified_name: Option<String>,
    pub view_qualified_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorldViewGraphModel {
    Ready {
        #[serde(rename = "graphBackgroundHex")]
        graph_background_hex: String,
        #[serde(rename = "graphPattern")]
        graph_pattern: WorldViewGraphPattern,
        clusters: Vec<WorldViewGraphCluster>,
        nodes: Vec<WorldViewGraphNode>,
        edges: Vec<WorldViewGraphEdge>,
        bounds: WorldViewGraphBounds,
    },
    Unavailable {
        reason: WorldViewGraphUnavailableReason,
    },
    Empty {
        reason: WorldViewGraphEmptyReason,
        #[serde(
            rename = "graphBackgroundHex",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        graph_background_hex: Option<String>,
        #[serde(
            rename = "graphPattern",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        graph_pattern: Option<WorldViewGraphPattern>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldViewGraphPattern {
    None,
    Grid,
    Dots,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldViewGraphUnavailableReason {
    MissingRealm,
    MissingScope,
    MissingView,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldViewGraphEmptyReason {
    NoPreferences,
    NoVisiblePreferences,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewGraphBounds {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewGraphCluster {
    pub background_rgba: String,
    pub badge_background_rgba: String,
    pub badge_text_hex: String,
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewGraphNode {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_id: Option<String>,
    pub label: String,
    pub preference_qualified_name: String,
    pub status: WorldViewGraphNodeStatus,
    pub target_state: Option<WorldViewTargetState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness_restrictions: Option<Vec<WorldViewReadinessRestriction>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_ready: Option<bool>,
    pub is_leaf: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_cases: Option<Vec<WorldViewSignalOutput>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal_case_names: Option<Vec<String>>,
    pub fill_hex: String,
    pub border_hex: String,
    pub text_hex: String,
    pub deemphasis: Option<WorldViewLensDeemphasis>,
    pub effect: Option<WorldViewEffect>,
    pub position: WorldViewGraphPosition,
    pub size: WorldViewGraphSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldViewGraphNodeStatus {
    Default,
    Leaf,
    Ready,
    Done,
    Goal,
    Focus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldViewTargetState {
    Actionable,
    Implementing,
    Satisfied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldViewLensDeemphasis {
    Fade,
    Ghost,
    Hide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldViewEffect {
    Blur,
    Dreamy,
    Focused,
    Glow,
    Prismatic,
    Clear,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewGraphPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewGraphSize {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewGraphEdge {
    pub deemphasis: Option<WorldViewLensDeemphasis>,
    pub id: String,
    pub line_hex: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flowspace_qualified_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_qualified_names: Option<Vec<String>>,
    pub source_id: String,
    pub target_id: String,
    pub connection_type: WorldViewConnectionType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldViewConnectionType {
    Foundational,
    Alternative,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorldViewReadinessRestriction {
    Opening {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        form_qualified_name: Option<String>,
        next_refresh_at: Option<String>,
        opening: WorldViewOpening,
        preference_local_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preference_qualified_name: Option<String>,
        source: WorldViewRestrictionSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_appearance_key: Option<String>,
        verdict: WorldViewOpeningVerdict,
    },
    Hold {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        form_qualified_name: Option<String>,
        hold: WorldViewReadinessHold,
        mode: WorldViewReadinessHoldMode,
        next_refresh_at: Option<String>,
        opening: WorldViewOpening,
        preference_local_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preference_qualified_name: Option<String>,
        reason_preference_local_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason_preference_qualified_name: Option<String>,
        source: WorldViewRestrictionSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_appearance_key: Option<String>,
        verdict: WorldViewReadinessHoldVerdict,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldViewRestrictionSource {
    Preference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldViewOpeningVerdict {
    Dormant,
    Indeterminate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldViewReadinessHoldVerdict {
    Active,
    Indeterminate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewOpening {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<WorldViewOpeningPlace>,
    pub windows: Vec<WorldViewOpeningWindow>,
    pub zone: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewOpeningPlace {
    pub place: String,
    pub relation: WorldViewOpeningPlaceRelation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorldViewOpeningPlaceRelation {
    At,
    AwayFrom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewOpeningWindow {
    pub anchor: WorldViewOpeningAnchor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<WorldViewOpeningTimeSpan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorldViewOpeningAnchor {
    Daily,
    DateTime {
        at: String,
    },
    Interval {
        end: String,
        start: String,
    },
    Weekly {
        weekdays: Vec<WorldViewOpeningWeekday>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorldViewOpeningWeekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewOpeningTimeSpan {
    pub end: String,
    pub start: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewReadinessHold {
    pub kind: WorldViewReadinessHoldKind,
    pub mode: WorldViewReadinessHoldMode,
    pub scope: WorldViewReadinessHoldScope,
    pub window: WorldViewOpening,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldViewReadinessHoldKind {
    Timebox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorldViewReadinessHoldMode {
    WhileActive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorldViewReadinessHoldScope {
    Set {
        set: WorldViewSetTarget,
    },
    View {
        #[serde(rename = "viewQualifiedName")]
        view_qualified_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorldViewSetTarget {
    Space {
        space_qualified_name: String,
    },
    RealmDerived {
        realm_qualified_name: String,
        selector: WorldViewRealmSelector,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldViewRealmSelector {
    All,
    Home,
    Refs,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewSignalOutput {
    pub case_name: String,
    pub evidence: Vec<WorldViewSignalEvidence>,
    pub signal_name: String,
    pub meanings: Vec<WorldViewSignalMeaning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorldViewSignalMeaning {
    TargetState { state: WorldViewTargetState },
    CodexThread,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum WorldViewSignalEvidence {
    Slot {
        slot_name: String,
        slot_space_qualified_name: String,
    },
    Ready {
        ready: bool,
    },
    TypedForm {
        form_qualified_name: String,
        value: WorldViewTypedObject,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewTypedValueEntry {
    pub key: String,
    pub value: WorldViewTypedValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorldViewTypedObject {
    pub kind: WorldViewTypedObjectKind,
    pub entries: Vec<WorldViewTypedValueEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldViewTypedObjectKind {
    Object,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorldViewTypedValue {
    Object {
        entries: Vec<WorldViewTypedValueEntry>,
    },
    Array {
        items: Vec<WorldViewTypedValue>,
    },
    String {
        value: String,
    },
    Number {
        value: f64,
    },
    Boolean {
        value: bool,
    },
    Null,
    PreferenceRef {
        preference_qualified_name: String,
    },
    SpaceRef {
        space_qualified_name: String,
    },
    FormRef {
        form_qualified_name: String,
    },
    SetRef {
        set: WorldViewSetTarget,
    },
    FlowspaceRef {
        flowspace_qualified_name: String,
    },
    ViewRef {
        view_qualified_name: String,
    },
}
