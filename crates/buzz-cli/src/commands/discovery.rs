//! Agent Discovery search: `colony discovery search`.
//!
//! The command calls the relay's `POST /discovery/search` (the provider key and
//! the per-company monthly cap live on the relay) and, with `--save`, turns each
//! new place into a prospect record in the company's Sales channel exactly as
//! the desktop does. The request shape and the place-to-prospect mapping mirror
//! `desktop/src/features/discovery/discoverySearch.ts` and `discoveryMapping.ts`;
//! keep field names and limits identical when either side changes.

use std::collections::HashSet;

use buzz_core::business_records::{
    PartyRecord, ProspectAction, ProspectEvidence, ProspectQualification, ProspectRecordInput,
    ProspectStage, RecordAction, BUSINESS_RECORD_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::client::{BuzzClient, CappedPost};
use crate::commands::leads::{load_leads, CurrentLead};
use crate::error::CliError;
use crate::links::lead_link;
use crate::validate::sdk_err;

const SEARCH_PATH: &str = "/discovery/search";
/// The desktop's `SEARCH_PAGE_SIZE`.
const DEFAULT_LIMIT: u32 = 20;
const MAX_LIMIT: u32 = 50;
const DEFAULT_PAGES: u32 = 1;
const MAX_PAGES: u32 = 5;
/// Industry label for places an agent saves. The relay only needs it non-empty.
const AGENT_INDUSTRY: &str = "Local businesses";
const EVIDENCE_TITLE: &str = "Google Maps listing";

const MAX_NAME_BYTES: usize = 240;
const MAX_TAXONOMY_BYTES: usize = 200;
const MAX_WEBSITE_BYTES: usize = 2_048;
const MAX_LOCATION_BYTES: usize = 200;
const MAX_EMAIL_BYTES: usize = 254;
const MAX_PHONE_BYTES: usize = 50;
const MAX_EXTERNAL_ID_BYTES: usize = 128;
const MAX_EVIDENCE_EXCERPT_BYTES: usize = 2_000;

/// Countries the relay maps to a Google Maps region (mirrors the desktop list).
const KNOWN_COUNTRIES: [&str; 18] = [
    "South Africa",
    "United States",
    "United Kingdom",
    "Nigeria",
    "Kenya",
    "Namibia",
    "Botswana",
    "Zimbabwe",
    "Zambia",
    "Mozambique",
    "Australia",
    "Canada",
    "India",
    "Ireland",
    "New Zealand",
    "Germany",
    "France",
    "Netherlands",
];

pub async fn dispatch(cmd: crate::DiscoveryCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::DiscoveryCmd;
    match cmd {
        DiscoveryCmd::Search {
            query,
            location,
            country,
            limit,
            pages,
            industry,
            vertical,
            save,
        } => {
            let options = SearchOptions::new(&query, &location, country.as_deref(), limit, pages)?
                .with_labels(industry.as_deref(), vertical.as_deref());
            cmd_search(client, &options, save).await
        }
    }
}

/// A validated `colony discovery search` request.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchOptions {
    query: String,
    location: String,
    country: Option<String>,
    limit: u32,
    pages: u32,
    /// Industry label saved leads carry; `None` means [`AGENT_INDUSTRY`].
    industry: Option<String>,
    /// Business-type label saved leads carry; `None` means the query.
    vertical: Option<String>,
}

impl SearchOptions {
    /// Set the labels saved leads carry. A blank label counts as not given.
    fn with_labels(mut self, industry: Option<&str>, vertical: Option<&str>) -> Self {
        let label = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        self.industry = label(industry);
        self.vertical = label(vertical);
        self
    }

    /// The industry and vertical saved leads carry: the labels given with
    /// `--industry` and `--vertical`, else "Local businesses" and the query.
    fn labels(&self) -> (&str, &str) {
        (
            self.industry.as_deref().unwrap_or(AGENT_INDUSTRY),
            self.vertical.as_deref().unwrap_or(&self.query),
        )
    }

    fn new(
        query: &str,
        location: &str,
        country: Option<&str>,
        limit: Option<u32>,
        pages: Option<u32>,
    ) -> Result<Self, CliError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(CliError::Usage("--query must not be empty".into()));
        }
        let limit = limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(CliError::Usage(format!(
                "--limit must be 1 through {MAX_LIMIT}"
            )));
        }
        let pages = pages.unwrap_or(DEFAULT_PAGES);
        if !(1..=MAX_PAGES).contains(&pages) {
            return Err(CliError::Usage(format!(
                "--pages must be 1 through {MAX_PAGES}"
            )));
        }
        Ok(Self {
            query: query.to_owned(),
            location: location.trim().to_owned(),
            country: country
                .map(str::trim)
                .filter(|country| !country.is_empty())
                .map(str::to_owned),
            limit,
            pages,
            industry: None,
            vertical: None,
        })
    }
}

/// A place the relay returned, in the relay's field names.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Place {
    #[serde(default)]
    place_id: Option<String>,
    name: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    subtypes: Option<String>,
    #[serde(default)]
    address: Option<String>,
    #[serde(default)]
    city: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    phone: Option<String>,
    #[serde(default)]
    website: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    rating: Option<f64>,
    #[serde(default)]
    reviews: Option<f64>,
    #[serde(default)]
    latitude: Option<f64>,
    #[serde(default)]
    longitude: Option<f64>,
    #[serde(default)]
    maps_url: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    business_status: Option<String>,
}

/// One page of search results.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchPage {
    places: Vec<Place>,
    #[serde(default)]
    next_skip: Option<u32>,
}

/// The country the desktop passes for a location: only when the whole location
/// is exactly a country the relay knows.
fn country_for_location(location: &str) -> Option<String> {
    let normalized = location.trim().to_lowercase();
    KNOWN_COUNTRIES
        .iter()
        .find(|country| country.to_lowercase() == normalized)
        .map(|country| (*country).to_owned())
}

/// The request body for one page; the same JSON as the desktop's
/// `buildSearchRequest`, with an explicit `--country` taking priority.
fn build_search_request(options: &SearchOptions, skip: u32) -> Value {
    let country = options
        .country
        .clone()
        .or_else(|| country_for_location(&options.location));
    let mut body = json!({
        "query": options.query,
        "location": options.location,
        "limit": options.limit,
        "skip": skip,
        "language": "en",
    });
    if let Some(country) = country {
        body["country"] = json!(country);
    }
    body
}

/// "Discovery limit reached for this month, resets 2026-11-01".
fn limit_message(resets_at: Option<&str>) -> String {
    match resets_at.and_then(|value| value.get(..10)) {
        Some(date) => format!("Discovery limit reached for this month, resets {date}"),
        None => "Discovery limit reached for this month, resets at the start of next month".into(),
    }
}

/// What one request for a page of places brought back.
enum PageOutcome {
    Page(SearchPage),
    MonthlyLimit(Option<String>),
}

async fn fetch_page(client: &BuzzClient, body: &Value) -> Result<PageOutcome, CliError> {
    match client.post_json_authed_cap_aware(SEARCH_PATH, body).await? {
        CappedPost::MonthlyLimit { resets_at } => Ok(PageOutcome::MonthlyLimit(resets_at)),
        CappedPost::Body(text) => {
            serde_json::from_str(&text)
                .map(PageOutcome::Page)
                .map_err(|error| {
                    CliError::Other(format!(
                        "Discovery answered with an unreadable page: {error}"
                    ))
                })
        }
    }
}

// ---------------------------------------------------------------------------
// Place to prospect mapping (mirror of discoveryMapping.ts)
// ---------------------------------------------------------------------------

/// Trims, then cuts to at most `max_bytes` UTF-8 bytes without splitting a character.
fn clamp_bytes(value: &str, max_bytes: usize) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= max_bytes {
        return trimmed.to_owned();
    }
    let mut result = String::new();
    for character in trimmed.chars() {
        if result.len() + character.len_utf8() > max_bytes {
            break;
        }
        result.push(character);
    }
    result.trim().to_owned()
}

fn present(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// An https URL for `value`: http is upgraded, anything else is dropped.
fn https_url(value: &Option<String>, max_bytes: usize) -> Option<String> {
    let mut parsed = url::Url::parse(present(value)?).ok()?;
    if parsed.scheme() == "http" {
        parsed.set_scheme("https").ok()?;
    }
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return None;
    }
    let href = parsed.to_string();
    (href.len() <= max_bytes).then_some(href)
}

/// How business names are compared when deciding whether a lead already exists.
fn lead_name_key(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// A stable key for a place: its provider id, else its name and address.
fn place_key(place: &Place) -> String {
    if let Some(id) = present(&place.place_id) {
        return id.to_owned();
    }
    format!(
        "{}|{}",
        place.name.trim().to_lowercase(),
        place.address.as_deref().unwrap_or("").trim().to_lowercase()
    )
}

/// The desktop's `stableRecordId`: a UUID derived from the parts, so the same
/// input maps to the same record on every device and every retry.
fn stable_record_id(parts: &[&str]) -> Uuid {
    let digest = Sha256::digest(format!("colony:w10:{}", parts.join(":")).as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn place_prospect_id(place: &Place) -> Uuid {
    stable_record_id(&["discovery-place", &place_key(place)])
}

/// The SalesTeams quality score: 30 for a name, plus 20 phone, 15 address,
/// 20 website, 15 rating. At most 100.
fn fit_score(place: &Place) -> u8 {
    let mut score = 30;
    if present(&place.phone).is_some() {
        score += 20;
    }
    if present(&place.address).is_some() {
        score += 15;
    }
    if present(&place.website).is_some() {
        score += 20;
    }
    if place.rating.is_some() {
        score += 15;
    }
    score.min(100)
}

/// "City, State" when the place has a city, otherwise its full address.
fn location_summary(place: &Place) -> Option<String> {
    let summary = match present(&place.city) {
        Some(city) => match present(&place.state) {
            Some(state) if state.to_lowercase() != city.to_lowercase() => {
                format!("{city}, {state}")
            }
            _ => city.to_owned(),
        },
        None => present(&place.address)?.to_owned(),
    };
    Some(clamp_bytes(&summary, MAX_LOCATION_BYTES)).filter(|summary| !summary.is_empty())
}

fn evidence_excerpt(place: &Place) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(category) = present(&place.category).or_else(|| present(&place.subtypes)) {
        parts.push(category.to_owned());
    }
    if let Some(rating) = place.rating {
        let reviews = match place.reviews {
            Some(reviews) if reviews > 0.0 => {
                let count = reviews.round() as u64;
                format!(
                    " from {count} {}",
                    if count == 1 { "review" } else { "reviews" }
                )
            }
            _ => String::new(),
        };
        parts.push(format!("Rated {rating}{reviews}"));
    }
    if let Some(address) = present(&place.address) {
        parts.push(address.to_owned());
    }
    clamp_bytes(&parts.join(" · "), MAX_EVIDENCE_EXCERPT_BYTES)
}

/// The party external ids that tie a record to its source place and search.
fn external_ids_for(place: &Place, campaign_id: &str) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(place_id) = present(&place.place_id) {
        let id = format!("outscraper:{place_id}");
        if id.len() <= MAX_EXTERNAL_ID_BYTES {
            ids.push(id);
        }
    }
    let campaign = format!("campaign:{campaign_id}");
    if campaign.len() <= MAX_EXTERNAL_ID_BYTES {
        ids.push(campaign);
    }
    ids
}

/// The create action for a place, or `None` when it cannot become a valid
/// record (no name, or no usable industry or vertical).
fn map_place_to_prospect(
    place: &Place,
    prospect_id: Uuid,
    campaign_id: &str,
    industry: &str,
    vertical: &str,
    now_seconds: i64,
) -> Option<ProspectAction> {
    let display_name = clamp_bytes(&place.name, MAX_NAME_BYTES);
    let industry = clamp_bytes(industry, MAX_TAXONOMY_BYTES);
    let vertical = clamp_bytes(vertical, MAX_TAXONOMY_BYTES);
    if display_name.is_empty() || industry.is_empty() || vertical.is_empty() {
        return None;
    }
    let website = https_url(&place.website, MAX_WEBSITE_BYTES);
    let evidence_url = https_url(&place.maps_url, MAX_WEBSITE_BYTES).or_else(|| website.clone());
    let email = present(&place.email)
        .filter(|email| email.contains('@') && email.len() <= MAX_EMAIL_BYTES)
        .map(str::to_owned);
    let phone = present(&place.phone)
        .filter(|phone| phone.len() <= MAX_PHONE_BYTES)
        .map(str::to_owned);
    Some(ProspectAction {
        schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
        prospect_id,
        action: RecordAction::Create,
        expected_head_event_id: None,
        prospect: ProspectRecordInput {
            prospect_id,
            party: PartyRecord {
                party_id: prospect_id,
                party_type: "organization".into(),
                display_name,
                external_ids: external_ids_for(place, campaign_id),
            },
            industry,
            vertical,
            fit_score: Some(fit_score(place)),
            origin: Some(buzz_core::business_records::ProspectOrigin::Discovery),
            discovery_tags: Vec::new(),
            potential_monthly_value_minor: None,
            website,
            contact_name: None,
            location: location_summary(place),
            email,
            phone,
            evidence: evidence_url
                .map(|url| {
                    vec![ProspectEvidence {
                        title: EVIDENCE_TITLE.to_owned(),
                        url,
                        excerpt: evidence_excerpt(place),
                        observed_at: now_seconds,
                    }]
                })
                .unwrap_or_default(),
            last_verified_at: Some(now_seconds),
            qualification: ProspectQualification::Unreviewed,
            saved: false,
            stage: ProspectStage::Qualified,
            lost_reason: None,
        },
        activity: None,
    })
}

// ---------------------------------------------------------------------------
// Saving
// ---------------------------------------------------------------------------

/// What the company already has, for skipping places that are already leads.
#[derive(Debug, Default)]
struct ExistingLeads {
    prospect_ids: HashSet<Uuid>,
    external_ids: HashSet<String>,
    names: HashSet<String>,
}

impl ExistingLeads {
    fn from_leads(leads: &[CurrentLead]) -> Self {
        let mut existing = Self::default();
        for lead in leads {
            existing.prospect_ids.insert(lead.head.prospect_id);
            let party = &lead.head.prospect.party;
            existing
                .external_ids
                .extend(party.external_ids.iter().cloned());
            existing.names.insert(lead_name_key(&party.display_name));
        }
        existing
    }

    fn has(&self, place: &Place, prospect_id: Uuid) -> bool {
        self.prospect_ids.contains(&prospect_id)
            || present(&place.place_id)
                .is_some_and(|id| self.external_ids.contains(&format!("outscraper:{id}")))
            || self.names.contains(&lead_name_key(&place.name))
    }

    fn remember(&mut self, place: &Place, prospect_id: Uuid) {
        self.prospect_ids.insert(prospect_id);
        if let Some(id) = present(&place.place_id) {
            self.external_ids.insert(format!("outscraper:{id}"));
        }
        self.names.insert(lead_name_key(&place.name));
    }
}

/// Split places into those to save and a count of those already leads. A place
/// repeated within the batch counts once.
fn partition_new<'a>(
    places: &'a [Place],
    existing: &mut ExistingLeads,
) -> (Vec<(Uuid, &'a Place)>, usize) {
    let mut fresh = Vec::new();
    let mut skipped = 0;
    for place in places {
        let prospect_id = place_prospect_id(place);
        if existing.has(place, prospect_id) {
            skipped += 1;
        } else {
            existing.remember(place, prospect_id);
            fresh.push((prospect_id, place));
        }
    }
    (fresh, skipped)
}

/// Sales channel and tenant id a new lead is written under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SaveTarget {
    channel_id: Uuid,
    community_id: Uuid,
}

fn tag_values<'a>(event: &'a Value, name: &str) -> Vec<&'a str> {
    event
        .get("tags")
        .and_then(Value::as_array)
        .map(|tags| {
            tags.iter()
                .filter_map(Value::as_array)
                .filter(|tag| tag.first().and_then(Value::as_str) == Some(name))
                .filter_map(|tag| tag.get(1).and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

/// The private channel named Sales among channel metadata events (kind 39000).
/// With several, the lowest id wins so every caller picks the same one.
fn pick_sales_channel(metadata: &[Value]) -> Option<Uuid> {
    metadata
        .iter()
        .filter(|event| {
            let private = event
                .get("tags")
                .and_then(Value::as_array)
                .is_some_and(|tags| {
                    tags.iter().any(|tag| {
                        tag.as_array().is_some_and(|tag| {
                            tag.len() == 1 && tag.first().and_then(Value::as_str) == Some("private")
                        })
                    })
                });
            let sales = tag_values(event, "name")
                .first()
                .is_some_and(|name| name.trim().eq_ignore_ascii_case("sales"));
            private && sales
        })
        .filter_map(|event| {
            tag_values(event, "d")
                .first()
                .and_then(|d| Uuid::parse_str(d).ok())
        })
        .min()
}

/// The tenant id the relay names for this relay host in its "communities I
/// belong to" answer. `None` unless exactly one entry names the host.
fn pick_tenant_id(body: &str, relay_url: &str) -> Option<Uuid> {
    let authority = buzz_core::tenant::relay_url_authority(relay_url);
    if authority.is_empty() {
        return None;
    }
    let answer: Value = serde_json::from_str(body).ok()?;
    let mut matches = answer
        .get("communities")?
        .as_array()?
        .iter()
        .filter(|entry| {
            entry
                .get("normalized_host")
                .and_then(Value::as_str)
                .is_some_and(|host| host.to_lowercase() == authority)
        });
    let entry = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Uuid::parse_str(entry.get("id")?.as_str()?).ok()
}

/// Where new leads go: the channel and tenant id of existing leads when there
/// are any, otherwise the member's private Sales channel and the tenant id the
/// relay reports for this host.
async fn resolve_save_target(
    client: &BuzzClient,
    leads: &[CurrentLead],
) -> Result<SaveTarget, CliError> {
    if let Some(lead) = leads.first() {
        return Ok(SaveTarget {
            channel_id: lead.channel_id,
            community_id: lead.community_id,
        });
    }
    let public_key = client.keys().public_key().to_hex();
    let member_events = client
        .query_paginated(json!({ "kinds": [39002], "#p": [public_key] }), 500)
        .await?;
    let channel_ids: Vec<String> = member_events
        .iter()
        .filter_map(|event| tag_values(event, "d").first().map(|d| (*d).to_owned()))
        .collect();
    let not_ready = || {
        CliError::NotFound(
            "The private Sales channel is not set up yet. An owner or admin sets it up by \
             opening Leads in Colony once."
                .into(),
        )
    };
    if channel_ids.is_empty() {
        return Err(not_ready());
    }
    let metadata = client
        .query_paginated(json!({ "kinds": [39000], "#d": channel_ids }), 500)
        .await?;
    let channel_id = pick_sales_channel(&metadata).ok_or_else(not_ready)?;
    let communities = client
        .get_authed("/api/communities/mine?scope=member")
        .await?;
    let community_id = pick_tenant_id(&communities, client.relay_url()).ok_or_else(|| {
        CliError::Other("The relay did not name which business this is. Try again.".into())
    })?;
    Ok(SaveTarget {
        channel_id,
        community_id,
    })
}

/// True when the relay refused a create because the lead already exists.
fn is_already_exists(text: &str) -> bool {
    text.to_lowercase().contains("already exists")
}

enum SaveOutcome {
    Saved,
    AlreadyExisted,
}

async fn save_place(
    client: &BuzzClient,
    target: SaveTarget,
    action: &ProspectAction,
) -> Result<SaveOutcome, CliError> {
    let builder = buzz_sdk::business_records::build_prospect_action(
        target.community_id,
        target.channel_id,
        action,
    )
    .map_err(sdk_err)?;
    let event = client.sign_event(builder)?;
    match client.submit_event(event).await {
        Ok(response) => {
            let answer: Value = serde_json::from_str(&response).unwrap_or(Value::Null);
            if answer.get("accepted").and_then(Value::as_bool) == Some(false) {
                let message = answer
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("the relay refused the lead");
                return if is_already_exists(message) {
                    Ok(SaveOutcome::AlreadyExisted)
                } else {
                    Err(CliError::Other(format!("could not save a lead: {message}")))
                };
            }
            Ok(SaveOutcome::Saved)
        }
        Err(CliError::Relay { body, .. }) if is_already_exists(&body) => {
            Ok(SaveOutcome::AlreadyExisted)
        }
        Err(error) => Err(error),
    }
}

/// Result of saving a batch of places.
#[derive(Debug, Default)]
struct SaveSummary {
    saved: Vec<Value>,
    skipped_existing: usize,
}

impl SaveSummary {
    fn to_json(&self) -> Value {
        json!({
            "saved": self.saved.len(),
            "skipped_existing": self.skipped_existing,
            "leads": self.saved,
        })
    }
}

/// Save every new place, stopping at the first failure. Places saved before a
/// failure stay saved, and a retry skips them as already leads.
async fn save_places(
    client: &BuzzClient,
    options: &SearchOptions,
    places: &[Place],
    summary: &mut SaveSummary,
) -> Result<(), CliError> {
    let leads = load_leads(client).await?;
    let mut existing = ExistingLeads::from_leads(&leads);
    let (fresh, skipped) = partition_new(places, &mut existing);
    summary.skipped_existing += skipped;
    if fresh.is_empty() {
        return Ok(());
    }
    let target = resolve_save_target(client, &leads).await?;
    let campaign_id = stable_record_id(&[
        "agent-search",
        &options.query.to_lowercase(),
        &options.location.to_lowercase(),
    ])
    .to_string();
    let now_seconds = chrono::Utc::now().timestamp();
    let (industry, vertical) = options.labels();
    for (prospect_id, place) in fresh {
        let Some(action) = map_place_to_prospect(
            place,
            prospect_id,
            &campaign_id,
            industry,
            vertical,
            now_seconds,
        ) else {
            continue;
        };
        match save_place(client, target, &action).await? {
            SaveOutcome::Saved => summary.saved.push(json!({
                "leadId": prospect_id.to_string(),
                "reference": lead_link(prospect_id),
                "name": action.prospect.party.display_name,
            })),
            SaveOutcome::AlreadyExisted => summary.skipped_existing += 1,
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

async fn cmd_search(
    client: &BuzzClient,
    options: &SearchOptions,
    save: bool,
) -> Result<(), CliError> {
    let mut places: Vec<Place> = Vec::new();
    // Why paging stopped early. Places already fetched were paid for, so they
    // are still printed or saved before this is reported.
    let mut stopped: Option<CliError> = None;
    let mut skip = 0;
    for _ in 0..options.pages {
        match fetch_page(client, &build_search_request(options, skip)).await {
            Ok(PageOutcome::Page(page)) => {
                places.extend(page.places);
                match page.next_skip {
                    Some(next) => skip = next,
                    None => break,
                }
            }
            Ok(PageOutcome::MonthlyLimit(resets_at)) => {
                stopped = Some(CliError::LimitReached(limit_message(resets_at.as_deref())));
                break;
            }
            Err(error) => {
                stopped = Some(error);
                break;
            }
        }
    }
    if places.is_empty() {
        if let Some(error) = stopped {
            return Err(error);
        }
        println!(
            "{}",
            if save {
                SaveSummary::default().to_json()
            } else {
                json!([])
            }
        );
        return Ok(());
    }
    if !save {
        println!(
            "{}",
            serde_json::to_string(&places)
                .map_err(|error| CliError::Other(format!("could not print places: {error}")))?
        );
        return stopped.map_or(Ok(()), Err);
    }
    let mut summary = SaveSummary::default();
    let saved = save_places(client, options, &places, &mut summary).await;
    println!("{}", summary.to_json());
    saved?;
    stopped.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(location: &str, country: Option<&str>) -> SearchOptions {
        SearchOptions::new("guesthouses", location, country, None, None).expect("options")
    }

    fn place(place_id: Option<&str>, name: &str) -> Place {
        Place {
            place_id: place_id.map(str::to_owned),
            name: name.to_owned(),
            category: None,
            subtypes: None,
            address: None,
            city: None,
            state: None,
            country: None,
            phone: None,
            website: None,
            email: None,
            rating: None,
            reviews: None,
            latitude: None,
            longitude: None,
            maps_url: None,
            description: None,
            business_status: None,
        }
    }

    #[test]
    fn search_builds_request_like_desktop() {
        // Same JSON as `buildSearchRequest` in discoverySearch.ts: a city passes
        // no country, a location that is exactly a known country passes it, and
        // the page size is the desktop's 20.
        assert_eq!(
            build_search_request(&options(" Melville, Johannesburg ", None), 0),
            json!({
                "query": "guesthouses",
                "location": "Melville, Johannesburg",
                "limit": 20,
                "skip": 0,
                "language": "en",
            })
        );
        assert_eq!(
            build_search_request(&options("south africa", None), 40),
            json!({
                "query": "guesthouses",
                "location": "south africa",
                "country": "South Africa",
                "limit": 20,
                "skip": 40,
                "language": "en",
            })
        );
        // An explicit --country wins and is passed as given.
        assert_eq!(
            build_search_request(&options("Durban", Some(" za ")), 0)["country"],
            "za"
        );
    }

    #[test]
    fn save_uses_given_industry_and_vertical() {
        let given = options("Melville", None).with_labels(
            Some(" Tourism and Hospitality "),
            Some("Guesthouses and inns"),
        );
        let (industry, vertical) = given.labels();
        assert_eq!(
            (industry, vertical),
            ("Tourism and Hospitality", "Guesthouses and inns")
        );
        let lodge = place(Some("ChIJ-1"), "Lodge");
        let action = map_place_to_prospect(
            &lodge,
            place_prospect_id(&lodge),
            "c",
            industry,
            vertical,
            1,
        )
        .expect("maps");
        assert_eq!(action.prospect.industry, "Tourism and Hospitality");
        assert_eq!(action.prospect.vertical, "Guesthouses and inns");

        // Not given (or blank): industry "Local businesses", vertical = the query.
        for unset in [
            options("Melville", None),
            options("Melville", None).with_labels(Some("  "), Some("")),
        ] {
            assert_eq!(unset.labels(), ("Local businesses", "guesthouses"));
        }
        // Each label is independent.
        assert_eq!(
            options("Melville", None)
                .with_labels(None, Some("Inns"))
                .labels(),
            ("Local businesses", "Inns")
        );

        // The flags parse on the real command.
        use clap::Parser;
        let parsed = crate::Cli::try_parse_from([
            "buzz",
            "discovery",
            "search",
            "--query",
            "q",
            "--location",
            "l",
            "--industry",
            "Tourism and Hospitality",
            "--vertical",
            "Inns",
            "--save",
        ]);
        assert!(parsed.is_ok(), "{:?}", parsed.err());
    }

    #[test]
    fn search_options_bound_limit_and_pages() {
        for limit in [0, 51] {
            assert!(SearchOptions::new("a", "b", None, Some(limit), None).is_err());
        }
        for pages in [0, 6] {
            assert!(SearchOptions::new("a", "b", None, None, Some(pages)).is_err());
        }
        assert!(SearchOptions::new("a", "b", None, Some(50), Some(5)).is_ok());
        assert!(SearchOptions::new("  ", "b", None, None, None).is_err());
    }

    #[test]
    fn monthly_limit_maps_to_exit_code_2_with_message() {
        let message = limit_message(Some("2026-11-01T00:00:00Z"));
        assert_eq!(
            message,
            "Discovery limit reached for this month, resets 2026-11-01"
        );
        let error = CliError::LimitReached(message);
        assert_eq!(crate::error::exit_code(&error), 2);
        assert_eq!(
            error.to_string(),
            "Discovery limit reached for this month, resets 2026-11-01"
        );
        assert_eq!(
            limit_message(None),
            "Discovery limit reached for this month, resets at the start of next month"
        );
    }

    #[test]
    fn save_skips_existing_outscraper_ids() {
        let mut existing = ExistingLeads::default();
        existing.external_ids.insert("outscraper:ChIJ-have".into());
        existing.names.insert(lead_name_key("Already  Named Lodge"));
        let places = vec![
            place(Some("ChIJ-have"), "Known By Id"),
            place(Some("ChIJ-new"), "Fresh Guesthouse"),
            place(Some("ChIJ-other"), "already named lodge"),
            place(Some("ChIJ-new"), "Fresh Guesthouse"),
        ];
        let (fresh, skipped) = partition_new(&places, &mut existing);
        assert_eq!(
            skipped, 3,
            "one by id, one by name, one repeated in the batch"
        );
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].1.name, "Fresh Guesthouse");
        // A retry of the same batch finds the place already remembered.
        let (retry, retry_skipped) = partition_new(&places, &mut existing);
        assert!(retry.is_empty());
        assert_eq!(retry_skipped, 4);
    }

    #[test]
    fn prospect_ids_match_the_desktop() {
        // Vectors computed with `stableRecordId` in businessRecordContract.ts.
        assert_eq!(
            place_prospect_id(&place(Some("ChIJ-test"), "Anything")).to_string(),
            "1236a47c-eb99-5924-8bfe-523f03c05f34"
        );
        let mut by_name = place(None, "Form & Field");
        by_name.address = Some("12 Main Road".into());
        assert_eq!(
            place_prospect_id(&by_name).to_string(),
            "98ff4afe-a30b-56d4-9c12-179d77fcb67b"
        );
        assert_eq!(
            stable_record_id(&["agent-search", "guesthouses", "melville, johannesburg"])
                .to_string(),
            "fd9aa13c-5efa-5839-8f79-73f6d7e3dd0d"
        );
    }

    #[test]
    fn places_map_to_prospects_like_the_desktop() {
        let mut full = place(Some("ChIJ-test"), "  Form & Field  ");
        full.phone = Some("+27 21 000 0000".into());
        full.address = Some("12 Main Road, Woodstock".into());
        full.city = Some("Cape Town".into());
        full.state = Some("Western Cape".into());
        full.website = Some("http://form-field.example/".into());
        full.email = Some("hello@form-field.example".into());
        full.rating = Some(4.8);
        full.reviews = Some(103.0);
        full.category = Some("Home goods store".into());
        full.maps_url = Some("https://www.google.com/maps/place/?q=place_id:ChIJ-test".into());
        let id = place_prospect_id(&full);
        let action = map_place_to_prospect(
            &full,
            id,
            "camp-1",
            "Local businesses",
            "guesthouses",
            1_800_000_000,
        )
        .expect("maps");
        let prospect = &action.prospect;
        assert_eq!(action.action, RecordAction::Create);
        assert_eq!(action.expected_head_event_id, None);
        assert_eq!(prospect.party.display_name, "Form & Field");
        assert_eq!(
            prospect.party.external_ids,
            vec![
                "outscraper:ChIJ-test".to_string(),
                "campaign:camp-1".to_string()
            ]
        );
        assert_eq!(prospect.fit_score, Some(100));
        assert_eq!(
            prospect.website.as_deref(),
            Some("https://form-field.example/")
        );
        assert_eq!(
            prospect.location.as_deref(),
            Some("Cape Town, Western Cape")
        );
        assert_eq!(prospect.stage, ProspectStage::Qualified);
        assert_eq!(prospect.qualification, ProspectQualification::Unreviewed);
        assert_eq!(prospect.evidence.len(), 1);
        assert_eq!(
            prospect.evidence[0].excerpt,
            "Home goods store · Rated 4.8 from 103 reviews · 12 Main Road, Woodstock"
        );

        // A place with only a name is still a valid record.
        let bare = place(None, "Bare Lodge");
        let action =
            map_place_to_prospect(&bare, place_prospect_id(&bare), "c", "i", "v", 1).expect("maps");
        assert_eq!(action.prospect.fit_score, Some(30));
        assert!(action.prospect.evidence.is_empty());
        assert!(action.prospect.party.external_ids == vec!["campaign:c".to_string()]);
        // No usable name, no record.
        assert!(
            map_place_to_prospect(&place(None, "  "), Uuid::new_v4(), "c", "i", "v", 1).is_none()
        );
    }

    #[test]
    fn sales_channel_is_the_lowest_private_sales_id() {
        let meta = |id: &str, name: &str, visibility: &str| json!({ "tags": [["d", id], ["name", name], [visibility]] });
        let low = "00000000-0000-0000-0000-000000000001";
        let high = "00000000-0000-0000-0000-000000000002";
        let events = vec![
            meta(high, "Sales", "private"),
            meta(low, " sales ", "private"),
            meta("00000000-0000-0000-0000-000000000000", "Sales", "public"),
            meta("00000000-0000-0000-0000-000000000003", "Ops", "private"),
        ];
        assert_eq!(
            pick_sales_channel(&events),
            Some(Uuid::parse_str(low).unwrap())
        );
        assert_eq!(pick_sales_channel(&events[2..]), None);
    }

    #[test]
    fn tenant_id_needs_exactly_one_host_match() {
        let id = "11111111-1111-1111-1111-111111111111";
        let body = |entries: Value| json!({ "communities": entries }).to_string();
        let one = body(json!([
            { "id": id, "normalized_host": "Acme.Colony.Example" },
            { "id": "22222222-2222-2222-2222-222222222222", "normalized_host": "other.example" },
        ]));
        assert_eq!(
            pick_tenant_id(&one, "wss://acme.colony.example"),
            Some(Uuid::parse_str(id).unwrap())
        );
        assert_eq!(pick_tenant_id(&one, "wss://nobody.example"), None);
        let two = body(json!([
            { "id": id, "normalized_host": "acme.colony.example" },
            { "id": "22222222-2222-2222-2222-222222222222", "normalized_host": "acme.colony.example" },
        ]));
        assert_eq!(pick_tenant_id(&two, "wss://acme.colony.example"), None);
        assert_eq!(
            pick_tenant_id("not json", "wss://acme.colony.example"),
            None
        );
    }

    /// A one-route stub relay that answers every request with `status` and
    /// `body`, and counts the requests it served.
    async fn stub_relay(
        status: &'static str,
        body: &'static str,
    ) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let served = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = served.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut buffer = vec![0u8; 8192];
                let _ = stream.read(&mut buffer).await;
                counter.fetch_add(1, Ordering::SeqCst);
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });
        (format!("http://{address}"), served)
    }

    fn test_client(base: &str) -> BuzzClient {
        BuzzClient::new(base.to_string(), nostr::Keys::generate(), None, None).unwrap()
    }

    #[tokio::test]
    async fn monthly_limit_page_is_returned_at_once_not_retried() {
        use std::sync::atomic::Ordering;
        let (base, served) = stub_relay(
            "429 Too Many Requests",
            r#"{"error":"monthly_limit","resets_at":"2026-11-01T00:00:00Z"}"#,
        )
        .await;
        let body = build_search_request(&options("Melville", None), 0);
        let outcome = fetch_page(&test_client(&base), &body)
            .await
            .expect("outcome");
        match outcome {
            PageOutcome::MonthlyLimit(resets_at) => {
                assert_eq!(resets_at.as_deref(), Some("2026-11-01T00:00:00Z"));
            }
            PageOutcome::Page(_) => panic!("a 429 is not a page"),
        }
        assert_eq!(
            served.load(Ordering::SeqCst),
            1,
            "a used-up month is never retried"
        );
    }

    #[tokio::test]
    async fn pages_are_read_from_the_relay_answer() {
        let (base, _) = stub_relay(
            "200 OK",
            r#"{"provider":"outscraper","places":[{"placeId":"ChIJ-1","name":"One","rating":4.5,"reviews":3}],"estimatedCostUsd":0.003,"nextSkip":20}"#,
        )
        .await;
        let body = build_search_request(&options("Melville", None), 0);
        match fetch_page(&test_client(&base), &body)
            .await
            .expect("outcome")
        {
            PageOutcome::Page(page) => {
                assert_eq!(page.next_skip, Some(20));
                assert_eq!(page.places.len(), 1);
                assert_eq!(page.places[0].place_id.as_deref(), Some("ChIJ-1"));
                assert_eq!(page.places[0].reviews, Some(3.0));
            }
            PageOutcome::MonthlyLimit(_) => panic!("not a limit"),
        }
    }

    #[test]
    fn already_exists_is_recognised_in_relay_wording() {
        assert!(is_already_exists("conflict: record already exists"));
        assert!(is_already_exists("prospect party identity already exists"));
        assert!(!is_already_exists("invalid prospect"));
    }
}
