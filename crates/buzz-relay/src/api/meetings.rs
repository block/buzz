//! Relay-owned HiveTalk (LiveKit video) control-plane proxy.
//!
//! Buzz "Meetings" uses HiveTalk / LiveKit for live video conferencing. Unlike
//! the KLIPY GIF proxy, this module holds **no** provider credential: the
//! integration is per-user (see `RESEARCH/MEETINGS_HIVETALK_PLAN.md` D5). Every
//! meeting operation is authorized by the *member's own* HiveTalk-signed request
//! (a kind-27235 event, or a LiveKit JWT for moderation).
//!
//! This proxy exists for two reasons the client cannot solve alone:
//!   1. HiveTalk's signed endpoints send no `Access-Control-Allow-Origin`, so a
//!      desktop WebView `fetch` is blocked — the relay must forward.
//!   2. It lets the relay gate meetings behind the existing Buzz membership
//!      check (NIP-98 + `relay_members`) before anything reaches HiveTalk.
//!
//! Each request therefore carries **two** independent signatures:
//!   - the buzz-relay NIP-98 in `Authorization` / `X-Pubkey` — proves the caller
//!     is a member of this community;
//!   - the HiveTalk auth material in `X-Hivetalk-Authorization` (+
//!     `X-Hivetalk-Challenge` for the challenge flow), or inside the JSON body
//!     for `get-token` — proves entitlement to HiveTalk.
//!
//! The proxy forwards the **raw request body bytes** unmodified (HiveTalk's
//! `payload` tag is `sha256(rawBody)`; re-serializing JSON would break the
//! signature) and filters **every** response through a static allowlist, so
//! provider diagnostics and room metadata cannot cross the relay boundary. Media
//! (audio/video RTC) never transits the relay — the client connects to the
//! LiveKit SFU directly with the token from `get-token`.
//!
//! ## Why every endpoint is filtered
//!
//! Five endpoints (`/auth/challenge`, `/plans`, `/room-info`, `/rooms-by-pubkey`,
//! `/list-rooms`) used to be forwarded verbatim on the grounds that their
//! responses hold nothing sensitive. That is a claim about the provider's
//! *current* output, and it is not one the relay can keep: HiveTalk's schema for
//! `/api/room-info` is explicitly `additionalProperties: true`, and the client's
//! `normalizeRooms` / `normalizePlans` both spread `...record`, so anything new
//! upstream lands in client state rather than being ignored. The allowlists are
//! derived from HiveTalk's `openapi.yaml` (archived at
//! `RESEARCH/HIVETALK_OPENAPI.yaml`) unioned with the fields the client's own
//! types name; there is no pass-through variant of [`Filter`] to reach for.
//!
//! Disabled unless `BUZZ_HIVETALK_API_ROOT` is set; otherwise every route 404s
//! before doing any work and NIP-11 does not advertise the capability.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{RawQuery, State},
    http::{header, HeaderMap, StatusCode},
    response::Json,
};
use futures_util::StreamExt;
use serde_json::{Map, Value};

use buzz_auth::LimitType;

use crate::config::HivetalkConfig;
use crate::state::AppState;

use super::{api_error, bridge, relay_members};

pub(crate) const MEETINGS_PREFIX: &str = "/meetings";
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_UPSTREAM_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ARRAY_ELEMENTS: usize = 500;
const MAX_QUERY_BYTES: usize = 512;

/// How the caller's HiveTalk auth reaches the upstream request.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HivetalkAuth {
    /// Public HiveTalk endpoint — no HiveTalk auth is forwarded.
    None,
    /// Challenge flow: `X-Hivetalk-Authorization` → `Authorization`,
    /// `X-Hivetalk-Challenge` → `X-Challenge`.
    ChallengeHeaders,
    /// `get-token` — the signed event rides inside the JSON body, so no auth
    /// header is forwarded.
    InBody,
    /// Moderation — `X-Hivetalk-Authorization` (a `Bearer <livekit-jwt>` value)
    /// is copied verbatim to `Authorization`; no challenge header.
    AuthorizationHeader,
}

/// Response-body filtering strategy for a proxied endpoint.
///
/// Every proxied endpoint filters. There is no pass-through variant: HiveTalk's
/// own schema for `/api/room-info` is `additionalProperties: true`, so "this
/// response has nothing in it today" is not a property the relay can rely on
/// staying true across provider deploys.
#[derive(Clone, Copy)]
enum Filter {
    /// Object response: keep only these keys, plus the standard error envelope.
    Object(&'static [&'static str]),
    /// Array-of-objects response: cap the length, then allowlist each element.
    /// A non-array body means an error status (these endpoints answer errors
    /// with an object), so it degrades to the error envelope alone.
    Array(&'static [&'static str]),
    /// Object carrying an array of objects under `array_key` — both levels are
    /// allowlisted, because `Object` alone would copy the nested elements whole.
    Envelope {
        fields: &'static [&'static str],
        array_key: &'static str,
        item_fields: &'static [&'static str],
    },
}

/// One proxied HiveTalk endpoint.
#[derive(Clone, Copy)]
struct Proxied {
    /// Path suffix after [`MEETINGS_PREFIX`], e.g. `/subscribe`.
    local: &'static str,
    /// Path on the HiveTalk API, e.g. `/api/subscribe`.
    upstream: &'static str,
    /// `true` = POST (body forwarded), `false` = GET.
    post: bool,
    auth: HivetalkAuth,
    /// Apply the dedicated `MeetingActions` rate-limit bucket in addition to the
    /// shared `ApiCalls` bucket. Set for money / quota / token operations.
    metered: bool,
    /// Query-string parameters that must be present (GET endpoints).
    required_query: &'static [&'static str],
    filter: Filter,
}

/// Fields common to moderation responses (most send no body).
const MOD_FIELDS: &[&str] = &["ok", "status", "room_name", "mute_on_join", "message"];
/// register-room / room-edit share this response shape. The first six entries
/// are the BOLT11 invoice fields HiveTalk returns on a `409 pending_invoice`
/// (the client resumes an in-flight subscription from them); every allowlist on
/// an endpoint that can 409 must carry them, which
/// `invoice_fields_reach_every_allowlist_that_can_409` enforces.
const ROOM_FIELDS: &[&str] = &[
    "amount_sats",
    "bolt11",
    "broadcast_pending",
    "created_via",
    "expires_at",
    "identifier",
    "intent_id",
    "lobby_enabled",
    "locked",
    "mute_on_join",
    "payment_hash",
    "plan",
    "pubkey",
    "room_description",
    "room_id",
    "room_name",
    "room_picture_url",
    "status",
    "updated_at",
];
/// `get-token` success shape (`token`/`url`) plus the invoice fields for a 409.
const GET_TOKEN_FIELDS: &[&str] = &[
    "amount_sats",
    "bolt11",
    "expires_at",
    "intent_id",
    "payment_hash",
    "plan",
    "token",
    "url",
];

/// `GET /api/auth/challenge`. The client destructures exactly these four.
const CHALLENGE_FIELDS: &[&str] = &["challenge", "domain", "expires_at", "nonce"];
/// `GET /api/room-info` (`RoomInfo`). The schema is `additionalProperties: true`
/// upstream, which is precisely why the relay pins the response instead of
/// forwarding whatever the provider decides to attach.
///
/// The documented schema names only `is_private` / `owner_pubkey` / `room_id` /
/// `room_name`, but the deployed provider answers with the registry row instead
/// (`status`, `pubkey`, `identifier`, and the moderation defaults) — captured
/// live from `l402relay.exe.xyz` on 2026-09-04, where a spec-only allowlist left
/// `room_name` as the single surviving key. So this is the union of both, on the
/// same reasoning as `ROOM_LIST_FIELDS`: a field in neither is a provider
/// addition, which is the thing being kept out.
const ROOM_INFO_FIELDS: &[&str] = &[
    // RoomInfo as documented.
    "is_private",
    "owner_pubkey",
    "room_id",
    "room_name",
    // The registry row the deployed provider actually returns.
    "audience_mode",
    "identifier",
    "lobby_enabled",
    "locked",
    "mute_on_join",
    "pubkey",
    "status",
    "updated_at",
    "username",
];
/// The `/api/plans` envelope around the `Plan` list.
const PLANS_ENVELOPE_FIELDS: &[&str] = &["free_quota", "plans"];
/// One `Plan` entry. HiveTalk names these `id` / `price_sats`; `normalizePlans`
/// in the client also accepts `plan` / `amount_sats` / `period` / `interval`, so
/// the allowlist carries both spellings rather than deciding for it.
const PLAN_FIELDS: &[&str] = &[
    "amount_sats",
    "can_record",
    "days",
    "id",
    "interval",
    "period",
    "plan",
    "price_sats",
    "room_quota",
];
/// One entry of `/api/list-rooms` (`RoomSummary`, live LiveKit rooms) or
/// `/api/rooms-by-pubkey` (`OwnedRoom`, registry rows).
///
/// One list for both because the client runs both through the same
/// `normalizeRooms`, which reads `name` **or** `room_name` and `numParticipants`
/// **or** `num_participants`. The contents are the union of the two upstream
/// schemas plus every key the client's own `ActiveRoom` type names — a field in
/// neither source is a provider addition, which is the thing being kept out.
const ROOM_LIST_FIELDS: &[&str] = &[
    // RoomSummary — live LiveKit rooms.
    "createdAt",
    "description",
    "name",
    "numParticipants",
    "pictureUrl",
    "sid",
    "status",
    // OwnedRoom — registered rooms.
    "audience_mode",
    "created_via",
    "identifier",
    "lobby_enabled",
    "locked",
    "mute_on_join",
    "pubkey",
    "room_description",
    "room_id",
    "room_name",
    "room_picture_url",
    "updated_at",
    // Registry columns the deployed provider returns but the schema omits,
    // captured live 2026-09-04.
    "username",
    // Spellings only the client names: `normalizeRooms` reads the first, and
    // `ActiveRoom` declares the second.
    "num_participants",
    "room_kind",
];

const ROUTE_CHALLENGE: Proxied = Proxied {
    local: "/auth/challenge",
    upstream: "/api/auth/challenge",
    post: false,
    auth: HivetalkAuth::None,
    metered: false,
    required_query: &[],
    filter: Filter::Object(CHALLENGE_FIELDS),
};
const ROUTE_PLANS: Proxied = Proxied {
    local: "/plans",
    upstream: "/api/plans",
    post: false,
    auth: HivetalkAuth::None,
    metered: false,
    required_query: &[],
    filter: Filter::Envelope {
        fields: PLANS_ENVELOPE_FIELDS,
        array_key: "plans",
        item_fields: PLAN_FIELDS,
    },
};
const ROUTE_SUBSCRIPTION: Proxied = Proxied {
    local: "/subscription",
    upstream: "/api/subscription",
    post: false,
    auth: HivetalkAuth::ChallengeHeaders,
    metered: false,
    required_query: &[],
    filter: Filter::Object(&[
        "can_record",
        "entitled",
        "free_quota",
        "grace_days",
        "in_grace",
        "paid_until",
        "plan",
        "pubkey",
        "room_quota",
        "rooms_in_use",
        "status",
    ]),
};
const ROUTE_SUBSCRIBE: Proxied = Proxied {
    local: "/subscribe",
    upstream: "/api/subscribe",
    post: true,
    auth: HivetalkAuth::ChallengeHeaders,
    metered: true,
    required_query: &[],
    filter: Filter::Object(&[
        "amount_sats",
        "bolt11",
        "expires_at",
        "intent_id",
        "payment_hash",
        "plan",
        "status",
    ]),
};
const ROUTE_PAYMENT_STATUS: Proxied = Proxied {
    local: "/payment/status",
    upstream: "/api/payment/status",
    post: false,
    auth: HivetalkAuth::ChallengeHeaders,
    // Read-only poll, not a write action. The client polls this every few
    // seconds for the multi-minute life of an invoice; the dedicated
    // `MeetingActions` bucket (default 10/min) is for subscribe / register-room
    // / get-token / moderation and would 429 the poll ~30s in. The general
    // per-principal HTTP limit still applies.
    metered: false,
    required_query: &["id"],
    filter: Filter::Object(&[
        "expires_at",
        "intent_id",
        "plan",
        "settled_msat",
        "status",
        "subscription",
    ]),
};
const ROUTE_REGISTER_ROOM: Proxied = Proxied {
    local: "/register-room",
    upstream: "/api/register-room",
    post: true,
    auth: HivetalkAuth::ChallengeHeaders,
    metered: true,
    required_query: &[],
    filter: Filter::Object(ROOM_FIELDS),
};
const ROUTE_ROOM_EDIT: Proxied = Proxied {
    local: "/room/edit",
    upstream: "/api/room/edit",
    post: true,
    auth: HivetalkAuth::ChallengeHeaders,
    metered: true,
    required_query: &[],
    filter: Filter::Object(ROOM_FIELDS),
};
const ROUTE_ROOM_DELETE: Proxied = Proxied {
    local: "/room/delete",
    upstream: "/api/room/delete",
    post: true,
    auth: HivetalkAuth::AuthorizationHeader,
    metered: true,
    required_query: &[],
    filter: Filter::Object(&["deleted", "events_removed", "livekit_deleted", "room_name"]),
};
const ROUTE_ROOM_INFO: Proxied = Proxied {
    local: "/room-info",
    upstream: "/api/room-info",
    post: false,
    auth: HivetalkAuth::None,
    metered: false,
    required_query: &["room_name"],
    filter: Filter::Object(ROOM_INFO_FIELDS),
};
const ROUTE_ROOMS_BY_PUBKEY: Proxied = Proxied {
    local: "/rooms-by-pubkey",
    upstream: "/api/rooms-by-pubkey",
    post: false,
    auth: HivetalkAuth::None,
    metered: false,
    required_query: &["pubkey"],
    filter: Filter::Array(ROOM_LIST_FIELDS),
};
const ROUTE_LIST_ROOMS: Proxied = Proxied {
    local: "/list-rooms",
    upstream: "/api/list-rooms",
    post: false,
    auth: HivetalkAuth::None,
    metered: false,
    required_query: &[],
    filter: Filter::Array(ROOM_LIST_FIELDS),
};
const ROUTE_GET_TOKEN: Proxied = Proxied {
    local: "/get-token",
    upstream: "/api/get-token",
    post: true,
    auth: HivetalkAuth::InBody,
    metered: true,
    required_query: &[],
    filter: Filter::Object(GET_TOKEN_FIELDS),
};

const fn moderation(local: &'static str, upstream: &'static str) -> Proxied {
    Proxied {
        local,
        upstream,
        post: true,
        auth: HivetalkAuth::AuthorizationHeader,
        metered: true,
        required_query: &[],
        filter: Filter::Object(MOD_FIELDS),
    }
}

const ROUTE_STAGE_PROMOTE: Proxied = moderation("/room/stage/promote", "/api/room/stage/promote");
const ROUTE_STAGE_DEMOTE: Proxied = moderation("/room/stage/demote", "/api/room/stage/demote");
const ROUTE_MOD_PROMOTE: Proxied =
    moderation("/room/moderator/promote", "/api/room/moderator/promote");
const ROUTE_MOD_DEMOTE: Proxied =
    moderation("/room/moderator/demote", "/api/room/moderator/demote");
const ROUTE_KICK_USER: Proxied = moderation("/kick-user", "/api/kick-user");
const ROUTE_MUTE_USER: Proxied = moderation("/mute-user", "/api/mute-user");
const ROUTE_NOTIFY_LOCK: Proxied = moderation("/room/notify-lock", "/api/room/notify-lock");
const ROUTE_MUTE_ON_JOIN: Proxied = moderation("/room/mute-on-join", "/api/room/mute-on-join");
const ROUTE_AUDIENCE_MODE: Proxied = moderation("/room/audience-mode", "/api/room/audience-mode");

/// Build the dedicated HiveTalk HTTP client. Redirects are disabled: a signed
/// request must never be replayed to an attacker-chosen `Location` host — a 3xx
/// comes back as a non-success status that [`forward`] maps to `502`.
pub fn build_hivetalk_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(UPSTREAM_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("static HiveTalk HTTP client configuration")
}

macro_rules! meeting_handler {
    ($(#[$meta:meta])* $name:ident, $route:expr) => {
        $(#[$meta])*
        pub async fn $name(
            State(state): State<Arc<AppState>>,
            headers: HeaderMap,
            RawQuery(query): RawQuery,
            body: axum::body::Bytes,
        ) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
            proxy(&state, &headers, query.as_deref(), body, $route).await
        }
    };
}

meeting_handler!(
    /// `GET /meetings/auth/challenge` → HiveTalk `GET /api/auth/challenge`.
    challenge, ROUTE_CHALLENGE
);
meeting_handler!(
    /// `GET /meetings/plans` → HiveTalk `GET /api/plans`.
    plans, ROUTE_PLANS
);
meeting_handler!(
    /// `GET /meetings/subscription` → HiveTalk `GET /api/subscription` (signed).
    subscription, ROUTE_SUBSCRIPTION
);
meeting_handler!(
    /// `POST /meetings/subscribe` → HiveTalk `POST /api/subscribe` (signed).
    subscribe, ROUTE_SUBSCRIBE
);
meeting_handler!(
    /// `GET /meetings/payment/status?id=` → HiveTalk `GET /api/payment/status` (signed).
    payment_status, ROUTE_PAYMENT_STATUS
);
meeting_handler!(
    /// `POST /meetings/register-room` → HiveTalk `POST /api/register-room` (signed).
    register_room, ROUTE_REGISTER_ROOM
);
meeting_handler!(
    /// `POST /meetings/room/edit` → HiveTalk `POST /api/room/edit` (signed).
    room_edit, ROUTE_ROOM_EDIT
);
meeting_handler!(
    /// `POST /meetings/room/delete` → HiveTalk `POST /api/room/delete` (LiveKit JWT).
    room_delete, ROUTE_ROOM_DELETE
);
meeting_handler!(
    /// `GET /meetings/room-info?room_name=` → HiveTalk `GET /api/room-info`.
    room_info, ROUTE_ROOM_INFO
);
meeting_handler!(
    /// `GET /meetings/rooms-by-pubkey?pubkey=` → HiveTalk `GET /api/rooms-by-pubkey`.
    rooms_by_pubkey, ROUTE_ROOMS_BY_PUBKEY
);
meeting_handler!(
    /// `GET /meetings/list-rooms` → HiveTalk `GET /api/list-rooms`.
    list_rooms, ROUTE_LIST_ROOMS
);
meeting_handler!(
    /// `POST /meetings/get-token` → HiveTalk `POST /api/get-token` (signed event in body).
    get_token, ROUTE_GET_TOKEN
);
meeting_handler!(
    /// Moderation proxy (LiveKit JWT): promote a participant to stage.
    stage_promote, ROUTE_STAGE_PROMOTE
);
meeting_handler!(
    /// Moderation proxy (LiveKit JWT): demote a participant from stage.
    stage_demote, ROUTE_STAGE_DEMOTE
);
meeting_handler!(
    /// Moderation proxy (LiveKit JWT): grant a participant moderator rights.
    moderator_promote, ROUTE_MOD_PROMOTE
);
meeting_handler!(
    /// Moderation proxy (LiveKit JWT): revoke a participant's moderator rights.
    moderator_demote, ROUTE_MOD_DEMOTE
);
meeting_handler!(
    /// Moderation proxy (LiveKit JWT): remove a participant from the room.
    kick_user, ROUTE_KICK_USER
);
meeting_handler!(
    /// Moderation proxy (LiveKit JWT): server-mute a participant.
    mute_user, ROUTE_MUTE_USER
);
meeting_handler!(
    /// Moderation proxy (LiveKit JWT): toggle the room lock.
    notify_lock, ROUTE_NOTIFY_LOCK
);
meeting_handler!(
    /// Moderation proxy (LiveKit JWT): toggle mute-on-join for the room.
    mute_on_join, ROUTE_MUTE_ON_JOIN
);
meeting_handler!(
    /// Moderation proxy (LiveKit JWT): toggle audience mode for the room.
    audience_mode, ROUTE_AUDIENCE_MODE
);

async fn proxy(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    query: Option<&str>,
    body: axum::body::Bytes,
    route: Proxied,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let Some(config) = state.config.hivetalk.as_ref() else {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "meetings is not configured",
        ));
    };

    let query = query.filter(|q| !q.is_empty());
    validate_query(&route, query)?;

    let (tenant, pubkey) = authenticate(state, headers, &route, query, &body).await?;
    if route.metered {
        enforce_meeting_admission(state, &tenant, &pubkey).await?;
    }

    forward(state, config, &route, headers, query, body).await
}

fn validate_query(route: &Proxied, query: Option<&str>) -> Result<(), (StatusCode, Json<Value>)> {
    let query = query.unwrap_or("");
    if query.len() > MAX_QUERY_BYTES {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "query string is too long",
        ));
    }
    for required in route.required_query {
        let present = query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .any(|(key, value)| key == *required && !value.is_empty());
        if !present {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                &format!("missing required query parameter `{required}`"),
            ));
        }
    }
    Ok(())
}

async fn authenticate(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    route: &Proxied,
    query: Option<&str>,
    body: &axum::body::Bytes,
) -> Result<(buzz_core::TenantContext, nostr::PublicKey), (StatusCode, Json<Value>)> {
    let raw_host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let tenant = crate::tenant::bind_community(&state.db, raw_host)
        .await
        .map_err(|_| {
            api_error(
                StatusCode::NOT_FOUND,
                "relay: no community is configured for this host",
            )
        })?;

    // The buzz-relay NIP-98 covers the exact relay URL the client is hitting,
    // including the query string for signed GETs (`normalize_url` keeps it).
    let mut path = format!("{MEETINGS_PREFIX}{}", route.local);
    if let Some(q) = query {
        path.push('?');
        path.push_str(q);
    }
    let method = if route.post { "POST" } else { "GET" };
    let expected_url = bridge::nip98_expected_url(&state.config.relay_url, &tenant, &path);
    let body_opt = if route.post {
        Some(body.as_ref())
    } else {
        None
    };
    let (pubkey, event_id_bytes) = bridge::verify_bridge_auth_with_options(
        headers,
        method,
        &expected_url,
        body_opt,
        true,
        route.post,
    )?;
    bridge::enforce_http_admission(state, &tenant, &pubkey).await?;
    bridge::check_nip98_replay(state, &tenant, event_id_bytes).await?;
    relay_members::enforce_relay_membership(
        state,
        tenant.community(),
        &pubkey.to_bytes(),
        headers
            .get("x-auth-tag")
            .and_then(|value| value.to_str().ok()),
    )
    .await?;

    Ok((tenant, pubkey))
}

async fn enforce_meeting_admission(
    state: &AppState,
    tenant: &buzz_core::TenantContext,
    pubkey: &nostr::PublicKey,
) -> Result<(), (StatusCode, Json<Value>)> {
    let limit = state.auth.config().rate_limits.meeting_actions_per_min;
    match crate::admission::check_principal(
        state.admission_rate_limiter.as_ref(),
        tenant,
        pubkey,
        LimitType::MeetingActions,
        60,
        limit,
    )
    .await
    {
        Ok(()) => Ok(()),
        Err(crate::admission::AdmissionError::Exceeded { reset_in_secs }) => {
            metrics::counter!("buzz_meeting_action_rejections_total", "reason" => "quota")
                .increment(1);
            Err(api_error(
                StatusCode::TOO_MANY_REQUESTS,
                &format!("rate-limited: meeting action quota exceeded; retry in {reset_in_secs}s"),
            ))
        }
        Err(crate::admission::AdmissionError::Unavailable) => Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "rate-limited: meeting action admission unavailable",
        )),
    }
}

async fn forward(
    state: &AppState,
    config: &HivetalkConfig,
    route: &Proxied,
    headers: &HeaderMap,
    query: Option<&str>,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let mut url = format!("{}{}", config.api_root, route.upstream);
    if let Some(q) = query {
        url.push('?');
        url.push_str(q);
    }

    let client = &state.hivetalk_http_client;
    let mut request = if route.post {
        client.post(&url)
    } else {
        client.get(&url)
    };

    match route.auth {
        HivetalkAuth::None | HivetalkAuth::InBody => {}
        HivetalkAuth::ChallengeHeaders => {
            let authorization = required_header(headers, "x-hivetalk-authorization")?;
            let challenge = required_header(headers, "x-hivetalk-challenge")?;
            request = request
                .header(header::AUTHORIZATION, authorization)
                .header("x-challenge", challenge);
        }
        HivetalkAuth::AuthorizationHeader => {
            let authorization = required_header(headers, "x-hivetalk-authorization")?;
            request = request.header(header::AUTHORIZATION, authorization);
        }
    }

    if route.post {
        // Raw bytes only — the HiveTalk signature covers the exact body.
        request = request
            .header(header::CONTENT_TYPE, "application/json")
            .body(body);
    }

    let response = request
        .timeout(UPSTREAM_TIMEOUT)
        .send()
        .await
        .map_err(|error| {
            tracing::warn!(
                timeout = error.is_timeout(),
                "HiveTalk upstream request failed"
            );
            api_error(StatusCode::BAD_GATEWAY, "meeting provider is unavailable")
        })?;

    let status = response.status();
    if status.is_redirection() {
        tracing::warn!(
            status = status.as_u16(),
            "HiveTalk upstream returned a redirect"
        );
        return Err(api_error(
            StatusCode::BAD_GATEWAY,
            "meeting provider is unavailable",
        ));
    }

    let upstream = limited_json(status.as_u16(), response).await?;
    let filtered = filter_body(route.filter, upstream);
    // Preserve the upstream status so the client can branch on 402/403/409.
    let mapped = StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    Ok((mapped, Json(filtered)))
}

fn required_header<'a>(
    headers: &'a HeaderMap,
    name: &str,
) -> Result<&'a str, (StatusCode, Json<Value>)> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, &format!("missing `{name}` header")))
}

/// Standard HiveTalk error-envelope fields (documented and client-renderable).
const ERROR_ENVELOPE: &[&str] = &[
    "error",
    "reason",
    "plans",
    "subscribe_api",
    "subscribe_url",
    "message",
];

fn filter_body(filter: Filter, upstream: Value) -> Value {
    match filter {
        Filter::Object(fields) => pick_fields(&upstream, fields),
        Filter::Array(item_fields) => match upstream {
            Value::Array(items) => Value::Array(
                items
                    .into_iter()
                    .take(MAX_ARRAY_ELEMENTS)
                    .map(|item| pick_fields(&item, item_fields))
                    .collect(),
            ),
            // Not an array: an error status, or `limited_json`'s plain-text
            // wrapper. Keep the envelope, drop everything else.
            other => pick_fields(&other, &[]),
        },
        Filter::Envelope {
            fields,
            array_key,
            item_fields,
        } => {
            let mut out = pick_fields(&upstream, fields);
            if let Some(Value::Array(items)) = out.get_mut(array_key) {
                *items = std::mem::take(items)
                    .into_iter()
                    .take(MAX_ARRAY_ELEMENTS)
                    .map(|item| pick_fields(&item, item_fields))
                    .collect();
            }
            out
        }
    }
}

fn pick_fields(value: &Value, fields: &[&str]) -> Value {
    let Some(object) = value.as_object() else {
        return Value::Object(Map::new());
    };
    let mut out = Map::new();
    for key in fields.iter().chain(ERROR_ENVELOPE.iter()) {
        if let Some(found) = object.get(*key) {
            out.insert((*key).to_string(), found.clone());
        }
    }
    Value::Object(out)
}

/// Longest plain-text upstream error forwarded to the client. Long enough for
/// HiveTalk's short messages, short enough that an HTML error page or a stack
/// trace cannot ride along.
const MAX_PLAIN_TEXT_ERROR_CHARS: usize = 200;

/// Render a non-JSON upstream error body as an `error` string.
///
/// Refuses anything that looks like markup and anything empty, so a provider
/// error page degrades to a generic message rather than leaking a document.
fn plain_text_error(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.starts_with('<') {
        return "meeting provider returned an error".to_string();
    }
    trimmed.chars().take(MAX_PLAIN_TEXT_ERROR_CHARS).collect()
}

/// Read the upstream body as JSON.
///
/// HiveTalk does not answer exclusively in JSON: `/api/room-info` reports an
/// unknown room as plain-text `404 Room not found`, and `/api/get-token`
/// documents several plain-text `400`s. Rewriting those to a `502` discarded
/// the real status and told the user the provider was down when it was working
/// correctly. A non-JSON body on an **error** status is wrapped as
/// `{"error": "<text>"}` so the caller keeps the status; a non-JSON body on a
/// **success** status is still a broken response and stays a `502`.
async fn limited_json(
    status: u16,
    response: reqwest::Response,
) -> Result<Value, (StatusCode, Json<Value>)> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_UPSTREAM_RESPONSE_BYTES as u64)
    {
        return Err(api_error(
            StatusCode::BAD_GATEWAY,
            "meeting provider response was too large",
        ));
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| {
            api_error(
                StatusCode::BAD_GATEWAY,
                "meeting provider response could not be read",
            )
        })?;
        if body.len().saturating_add(chunk.len()) > MAX_UPSTREAM_RESPONSE_BYTES {
            return Err(api_error(
                StatusCode::BAD_GATEWAY,
                "meeting provider response was too large",
            ));
        }
        body.extend_from_slice(&chunk);
    }

    if body.is_empty() {
        // Moderation endpoints legitimately return an empty 200 body.
        return Ok(Value::Object(Map::new()));
    }

    match serde_json::from_slice(&body) {
        Ok(value) => Ok(value),
        Err(_) if (400..600).contains(&status) => {
            let message = plain_text_error(&body);
            tracing::debug!(
                status,
                "HiveTalk upstream returned a non-JSON error body; forwarding it"
            );
            Ok(Value::Object(Map::from_iter([(
                "error".to_string(),
                Value::String(message),
            )])))
        }
        Err(_) => Err(api_error(
            StatusCode::BAD_GATEWAY,
            "meeting provider returned an invalid response",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BOLT11 invoice fields HiveTalk returns on a `409 pending_invoice`. Not a
    /// production const: three allowlists spell these out inline, and this is
    /// the check that keeps those copies from drifting apart.
    const INVOICE_FIELDS: &[&str] = &[
        "amount_sats",
        "bolt11",
        "expires_at",
        "intent_id",
        "payment_hash",
        "plan",
    ];

    /// A dropped invoice field would silently break resume-an-invoice: the
    /// client 409s, the filter strips the `bolt11`, and the user sees a pending
    /// subscription with no QR to pay.
    #[test]
    fn invoice_fields_reach_every_allowlist_that_can_409() {
        let subscribe_fields = match ROUTE_SUBSCRIBE.filter {
            Filter::Object(fields) => fields,
            _ => panic!("/subscribe must filter its response object"),
        };
        for allowlist in [ROOM_FIELDS, GET_TOKEN_FIELDS, subscribe_fields] {
            for field in INVOICE_FIELDS {
                assert!(
                    allowlist.contains(field),
                    "allowlist {allowlist:?} is missing invoice field {field}"
                );
            }
        }
    }

    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::{get, post},
        Router,
    };
    use tower::ServiceExt;

    async fn test_state(configure_hivetalk: Option<&str>) -> Arc<AppState> {
        let mut config = crate::config::Config::from_env().expect("test config");
        config.klipy = None;
        config.hivetalk = configure_hivetalk.map(|root| HivetalkConfig {
            api_root: root.to_string(),
        });
        config.redis_url = "redis://127.0.0.1:1".to_string();

        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://buzz:buzz_dev@127.0.0.1:1/buzz") // sadscan:disable np.postgres.1
            .expect("lazy test database pool");
        let db = buzz_db::Db::from_pool(pool.clone());
        let redis_pool = deadpool_redis::Config::from_url(&config.redis_url)
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))
            .expect("lazy test Redis pool");
        let pubsub = Arc::new(
            buzz_pubsub::PubSubManager::new(&config.redis_url, redis_pool.clone())
                .await
                .expect("test pubsub"),
        );
        let auth = buzz_auth::AuthService::new(config.auth.clone());
        let search = buzz_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(buzz_workflow::WorkflowEngine::new(
            db.clone(),
            buzz_workflow::WorkflowConfig::default(),
        ));
        let media_storage =
            buzz_media::MediaStorage::new(&config.media).expect("test media storage config");
        let (state, _audit_shutdown) = AppState::new(
            config,
            db,
            redis_pool,
            None::<buzz_audit::AuditService>,
            pubsub,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        Arc::new(state)
    }

    fn router(state: Arc<AppState>) -> Router {
        Router::new()
            .route("/meetings/auth/challenge", get(challenge))
            .route("/meetings/plans", get(plans))
            .route("/meetings/subscribe", post(subscribe))
            .route("/meetings/payment/status", get(payment_status))
            .route("/meetings/get-token", post(get_token))
            .with_state(state)
    }

    #[tokio::test]
    async fn routes_return_404_before_auth_when_unconfigured() {
        let state = test_state(None).await;
        let response = router(state)
            .oneshot(
                Request::post("/meetings/subscribe")
                    .body(Body::from("{}"))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn configured_route_rejects_request_without_buzz_auth() {
        // Configured, but the request carries neither a resolvable tenant Host
        // nor a NIP-98 signature: it must be rejected as a client error and
        // never forwarded upstream.
        let state = test_state(Some("http://127.0.0.1:1")).await;
        let response = router(state)
            .oneshot(
                Request::post("/meetings/subscribe")
                    .body(Body::from("{}"))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert!(response.status().is_client_error());
    }

    #[tokio::test]
    async fn missing_required_query_param_is_400_before_forward() {
        let state = test_state(Some("http://127.0.0.1:1")).await;
        // payment/status needs `?id=`; the request has no NIP-98 either, but the
        // query check runs first.
        let response = router(state)
            .oneshot(
                Request::get("/meetings/payment/status")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn filter_object_keeps_allowlisted_and_envelope_fields_only() {
        let filtered = filter_body(
            ROUTE_SUBSCRIBE.filter,
            serde_json::json!({
                "bolt11": "lnbc1...",
                "amount_sats": 360,
                "intent_id": "abc",
                "status": "pending",
                "internal_debug": "secret-diagnostics",
                "error": "nope",
            }),
        );
        assert_eq!(
            filtered,
            serde_json::json!({
                "bolt11": "lnbc1...",
                "amount_sats": 360,
                "intent_id": "abc",
                "status": "pending",
                "error": "nope",
            })
        );
        assert!(!filtered.to_string().contains("internal_debug"));
    }

    /// The `RoomInfo` schema and the deployed provider disagree: the live
    /// response is the registry row, not the four documented keys. Body captured
    /// verbatim from `GET https://l402relay.exe.xyz/api/room-info` on 2026-09-04.
    /// A spec-only allowlist reduced this to `{"room_name": ...}`.
    #[test]
    fn filter_object_keeps_the_live_room_info_row() {
        let filtered = filter_body(
            ROUTE_ROOM_INFO.filter,
            serde_json::json!({
                "room_name": "buzz-meet-spike",
                "status": "private",
                "updated_at": "2026-08-21T01:13:17Z",
                "identifier": "buzz-meet-spike",
                "pubkey": "1f0cb6f1e65c98b224b385f9f7c7a11d2bfb3f40a06ae599a2081ecc4b0fd2dd",
                "username": "buzz-spike",
                "locked": false,
                "lobby_enabled": false,
                "mute_on_join": false,
                "audience_mode": false,
                "sfu_node": "internal-diagnostic",
            }),
        );
        let object = filtered.as_object().expect("object");
        for key in [
            "room_name",
            "status",
            "updated_at",
            "identifier",
            "pubkey",
            "username",
            "locked",
            "lobby_enabled",
            "mute_on_join",
            "audience_mode",
        ] {
            assert!(object.contains_key(key), "allowlist dropped live key {key}");
        }
        assert!(
            !filtered.to_string().contains("sfu_node"),
            "provider addition survived the allowlist"
        );
    }

    #[test]
    fn filter_array_caps_length_and_allowlists_each_element() {
        let big: Vec<Value> = (0..(MAX_ARRAY_ELEMENTS + 50))
            .map(|i| serde_json::json!({ "name": format!("room-{i}"), "host_ip": "10.0.0.1" }))
            .collect();
        let filtered = filter_body(ROUTE_LIST_ROOMS.filter, Value::Array(big));
        let items = filtered.as_array().expect("array");
        assert_eq!(items.len(), MAX_ARRAY_ELEMENTS);
        assert_eq!(items[0]["name"], "room-0");
        assert!(
            !filtered.to_string().contains("host_ip"),
            "per-element allowlist did not apply"
        );
    }

    /// The five endpoints that used to forward their response verbatim. A
    /// provider that starts attaching diagnostics — or room metadata a member
    /// should not see — must not reach the client through any of them.
    #[test]
    fn every_formerly_public_endpoint_drops_unknown_fields() {
        let list = filter_body(
            ROUTE_LIST_ROOMS.filter,
            serde_json::json!([{ "name": "standup", "numParticipants": 3, "sfu_node": "leak" }]),
        );
        assert_eq!(
            list,
            serde_json::json!([{ "name": "standup", "numParticipants": 3 }])
        );

        let owned = filter_body(
            ROUTE_ROOMS_BY_PUBKEY.filter,
            serde_json::json!([{ "room_name": "mine", "locked": true, "owner_email": "leak" }]),
        );
        assert_eq!(
            owned,
            serde_json::json!([{ "room_name": "mine", "locked": true }])
        );

        let info = filter_body(
            ROUTE_ROOM_INFO.filter,
            serde_json::json!({
                "room_name": "standup",
                "is_private": false,
                "internal_notes": "leak",
            }),
        );
        assert_eq!(
            info,
            serde_json::json!({ "room_name": "standup", "is_private": false })
        );

        let challenge = filter_body(
            ROUTE_CHALLENGE.filter,
            serde_json::json!({ "challenge": "jwt", "nonce": "n", "server_secret": "leak" }),
        );
        assert_eq!(
            challenge,
            serde_json::json!({ "challenge": "jwt", "nonce": "n" })
        );
    }

    /// `/plans` is an envelope, so the elements need filtering too — an
    /// envelope-only allowlist copies each `Plan` object whole.
    #[test]
    fn filter_envelope_allowlists_the_nested_plan_entries() {
        let filtered = filter_body(
            ROUTE_PLANS.filter,
            serde_json::json!({
                "free_quota": 1,
                "internal_pricing_engine": "leak",
                "plans": [
                    { "id": "standard_1y", "price_sats": 21_000, "cost_basis_msat": "leak" },
                ],
            }),
        );
        assert_eq!(
            filtered,
            serde_json::json!({
                "free_quota": 1,
                "plans": [{ "id": "standard_1y", "price_sats": 21_000 }],
            })
        );
    }

    /// These endpoints answer errors with an object, not an array: `/room-info`
    /// 404s an unregistered name, which is how the client tells a permanent room
    /// from an ephemeral one. The status and the envelope must survive.
    #[test]
    fn filter_array_on_an_error_object_keeps_only_the_envelope() {
        let filtered = filter_body(
            ROUTE_LIST_ROOMS.filter,
            serde_json::json!({ "error": "Room not found", "stack": "leak" }),
        );
        assert_eq!(filtered, serde_json::json!({ "error": "Room not found" }));
    }

    fn upstream_response(status: u16, body: &'static str) -> reqwest::Response {
        reqwest::Response::from(
            axum::http::Response::builder()
                .status(status)
                .body(body)
                .expect("test upstream response"),
        )
    }

    #[test]
    fn plain_text_error_keeps_a_short_provider_message() {
        assert_eq!(plain_text_error(b"Room not found"), "Room not found");
        assert_eq!(
            plain_text_error(
                b"  pubkey is required
"
            ),
            "pubkey is required"
        );
    }

    #[test]
    fn plain_text_error_refuses_markup_and_empty_bodies() {
        let generic = "meeting provider returned an error";
        assert_eq!(plain_text_error(b"<html><body>oops</body></html>"), generic);
        assert_eq!(plain_text_error(b"   "), generic);
        assert_eq!(plain_text_error(b""), generic);
    }

    #[test]
    fn plain_text_error_caps_a_long_body() {
        let long = "x".repeat(MAX_PLAIN_TEXT_ERROR_CHARS + 50);
        assert_eq!(
            plain_text_error(long.as_bytes()).chars().count(),
            MAX_PLAIN_TEXT_ERROR_CHARS
        );
    }

    #[tokio::test]
    async fn limited_json_forwards_a_plain_text_error_body() {
        // HiveTalk answers `/api/room-info` for an unknown room with a
        // plain-text 404. Rewriting it to a 502 told the user the provider was
        // down; the caller needs the real status to say "no such room".
        let value = limited_json(404, upstream_response(404, "Room not found"))
            .await
            .expect("a non-JSON error body is forwarded, not rejected");
        assert_eq!(value["error"], Value::String("Room not found".to_string()));
    }

    #[tokio::test]
    async fn limited_json_still_rejects_a_non_json_success_body() {
        let error = limited_json(200, upstream_response(200, "not json"))
            .await
            .expect_err("a non-JSON success body is a broken response");
        assert_eq!(error.0, StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn limited_json_prefers_a_real_json_error_envelope() {
        let value = limited_json(
            402,
            upstream_response(402, r#"{"reason":"subscription_required"}"#),
        )
        .await
        .expect("valid JSON parses");
        assert_eq!(
            value["reason"],
            Value::String("subscription_required".to_string())
        );
    }

    #[test]
    fn filter_object_on_upstream_402_envelope_preserves_documented_fields() {
        let filtered = filter_body(
            ROUTE_REGISTER_ROOM.filter,
            serde_json::json!({
                "error": "subscription required",
                "reason": "subscription_required",
                "plans": [{ "plan": "standard_1y" }],
                "subscribe_url": "https://l402stage.exe.xyz/dashboard/subscribe",
                "trace_id": "leak-me",
            }),
        );
        assert_eq!(
            filtered,
            serde_json::json!({
                "error": "subscription required",
                "reason": "subscription_required",
                "plans": [{ "plan": "standard_1y" }],
                "subscribe_url": "https://l402stage.exe.xyz/dashboard/subscribe",
            })
        );
    }

    #[test]
    fn room_and_get_token_allowlists_carry_pending_invoice_fields() {
        // A `409 pending_invoice` can come back from register-room and get-token,
        // not just /subscribe; the client resumes the invoice from these fields.
        for field in INVOICE_FIELDS {
            assert!(
                ROOM_FIELDS.contains(field),
                "ROOM_FIELDS missing invoice field `{field}`"
            );
            assert!(
                GET_TOKEN_FIELDS.contains(field),
                "GET_TOKEN_FIELDS missing invoice field `{field}`"
            );
        }
    }

    #[test]
    fn get_token_409_pending_invoice_passes_the_invoice_through() {
        let filtered = filter_body(
            ROUTE_GET_TOKEN.filter,
            serde_json::json!({
                "reason": "pending_invoice",
                "intent_id": "int_1",
                "bolt11": "lnbc1...",
                "amount_sats": 21_000,
                "payment_hash": "hash",
                "expires_at": "2026-01-01T00:00:00Z",
                "plan": "bulk10_1y",
                "internal_trace": "leak-me",
            }),
        );
        assert_eq!(filtered["bolt11"], "lnbc1...");
        assert_eq!(filtered["intent_id"], "int_1");
        assert_eq!(filtered["reason"], "pending_invoice");
        assert!(!filtered.to_string().contains("internal_trace"));
    }

    #[test]
    fn route_local_paths_are_prefixed_and_unique() {
        let routes = [
            ROUTE_CHALLENGE,
            ROUTE_PLANS,
            ROUTE_SUBSCRIPTION,
            ROUTE_SUBSCRIBE,
            ROUTE_PAYMENT_STATUS,
            ROUTE_REGISTER_ROOM,
            ROUTE_ROOM_EDIT,
            ROUTE_ROOM_DELETE,
            ROUTE_ROOM_INFO,
            ROUTE_ROOMS_BY_PUBKEY,
            ROUTE_LIST_ROOMS,
            ROUTE_GET_TOKEN,
            ROUTE_STAGE_PROMOTE,
            ROUTE_STAGE_DEMOTE,
            ROUTE_MOD_PROMOTE,
            ROUTE_MOD_DEMOTE,
            ROUTE_KICK_USER,
            ROUTE_MUTE_USER,
            ROUTE_NOTIFY_LOCK,
            ROUTE_MUTE_ON_JOIN,
            ROUTE_AUDIENCE_MODE,
        ];
        let mut seen = std::collections::HashSet::new();
        for route in routes {
            assert!(route.local.starts_with('/'));
            assert!(route.upstream.starts_with("/api/"));
            assert!(
                seen.insert(route.local),
                "duplicate local path {}",
                route.local
            );
        }
    }

    #[tokio::test]
    async fn forwards_challenge_headers_and_raw_body_and_filters_response() {
        // Local mock HiveTalk upstream.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock upstream");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/api/subscribe",
                    post(|headers: HeaderMap, body: axum::body::Bytes| async move {
                        assert_eq!(
                            headers.get("authorization").and_then(|v| v.to_str().ok()),
                            Some("Nostr base64event")
                        );
                        assert_eq!(
                            headers.get("x-challenge").and_then(|v| v.to_str().ok()),
                            Some("jwt-challenge")
                        );
                        assert_eq!(body.as_ref(), br#"{"plan":"standard_1y"}"#);
                        (
                            [(header::CONTENT_TYPE, "application/json")],
                            r#"{"bolt11":"lnbc1","amount_sats":360,"provider_note":"hidden"}"#,
                        )
                    }),
                ),
            )
            .await
            .expect("serve mock upstream");
        });

        let config = HivetalkConfig {
            api_root: format!("http://{addr}"),
        };
        let route = ROUTE_SUBSCRIBE;
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-hivetalk-authorization",
            "Nostr base64event".parse().unwrap(),
        );
        headers.insert("x-hivetalk-challenge", "jwt-challenge".parse().unwrap());
        let state = test_state(Some(&config.api_root)).await;

        let (status, Json(body)) = forward(
            &state,
            &config,
            &route,
            &headers,
            None,
            axum::body::Bytes::from_static(br#"{"plan":"standard_1y"}"#),
        )
        .await
        .expect("forward ok");

        server.abort();
        let _ = server.await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            serde_json::json!({ "bolt11": "lnbc1", "amount_sats": 360 })
        );
        assert!(!body.to_string().contains("provider_note"));
    }

    #[tokio::test]
    async fn upstream_402_status_is_preserved_through_forward() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock upstream");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/api/register-room",
                    post(|| async {
                        (
                            StatusCode::PAYMENT_REQUIRED,
                            [(header::CONTENT_TYPE, "application/json")],
                            r#"{"error":"x","reason":"subscription_required","secret":"nope"}"#,
                        )
                    }),
                ),
            )
            .await
            .expect("serve mock upstream");
        });

        let config = HivetalkConfig {
            api_root: format!("http://{addr}"),
        };
        let mut headers = HeaderMap::new();
        headers.insert("x-hivetalk-authorization", "Nostr e".parse().unwrap());
        headers.insert("x-hivetalk-challenge", "j".parse().unwrap());
        let state = test_state(Some(&config.api_root)).await;

        let (status, Json(body)) = forward(
            &state,
            &config,
            &ROUTE_REGISTER_ROOM,
            &headers,
            None,
            axum::body::Bytes::from_static(b"{}"),
        )
        .await
        .expect("forward returns Ok with mapped status");

        server.abort();
        let _ = server.await;

        assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
        assert_eq!(body["reason"], "subscription_required");
        assert!(!body.to_string().contains("secret"));
    }

    #[tokio::test]
    async fn upstream_redirect_is_not_followed_and_maps_to_502() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock upstream");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/api/list-rooms",
                    get(|| async {
                        (
                            StatusCode::FOUND,
                            [(header::LOCATION, "http://attacker.example/leak")],
                            "",
                        )
                    }),
                ),
            )
            .await
            .expect("serve mock upstream");
        });

        let config = HivetalkConfig {
            api_root: format!("http://{addr}"),
        };
        let state = test_state(Some(&config.api_root)).await;
        let error = forward(
            &state,
            &config,
            &ROUTE_LIST_ROOMS,
            &HeaderMap::new(),
            None,
            axum::body::Bytes::new(),
        )
        .await
        .expect_err("redirect must be an error");

        server.abort();
        let _ = server.await;
        assert_eq!(error.0, StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn oversized_upstream_body_is_rejected() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock upstream");
        let addr = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route(
                    "/api/list-rooms",
                    get(|| async {
                        (
                            [(header::CONTENT_TYPE, "application/json")],
                            "x".repeat(MAX_UPSTREAM_RESPONSE_BYTES + 1),
                        )
                    }),
                ),
            )
            .await
            .expect("serve mock upstream");
        });

        let config = HivetalkConfig {
            api_root: format!("http://{addr}"),
        };
        let state = test_state(Some(&config.api_root)).await;
        let error = forward(
            &state,
            &config,
            &ROUTE_LIST_ROOMS,
            &HeaderMap::new(),
            None,
            axum::body::Bytes::new(),
        )
        .await
        .expect_err("oversized body rejected");

        server.abort();
        let _ = server.await;
        assert_eq!(error.0, StatusCode::BAD_GATEWAY);
    }
}
