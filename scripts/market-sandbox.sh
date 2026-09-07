#!/usr/bin/env bash
# Relay-backed two-agent sandbox for channel-rooted buzz-market/v0 envelopes.
set -euo pipefail

usage() {
  cat <<'TXT'
Usage:
  market-sandbox.sh create --actor NAME --title TEXT --summary TEXT [--direction offer|request] [--mechanism fixed|auction|reverse-auction|tender] [--quantity N|unlimited] [--price N] [--budget N] [--increment N] [--decrement N] [--delivery-minutes N] [--closes-at UNIX]
  market-sandbox.sh response --channel UUID --listing EVENT --actor NAME --quantity N [--amount N] --message TEXT
  market-sandbox.sh award --channel UUID --listing EVENT --response EVENT --actor NAME --quantity N --amount N
  market-sandbox.sh fulfill --channel UUID --listing EVENT --award EVENT --actor NAME --message TEXT
  market-sandbox.sh settle --channel UUID --listing EVENT --award EVENT --fulfillment EVENT --actor NAME --amount N
  market-sandbox.sh watch --channel UUID

BUZZ_RELAY_URL and BUZZ_PRIVATE_KEY select the relay and signing agent. `create`
creates an open stream channel, writes its canonical contract message, then
publishes one kind:1 Pulse announcement pointing back to that event.
TXT
}

die() { echo "market-sandbox: $*" >&2; exit 1; }
need() { [[ -n "${2:-}" ]] || die "missing --$1"; }
channel_write() { buzz messages send --channel "$channel" --content "$1"; }

command_name="${1:-}"
[[ -n "$command_name" ]] || { usage; exit 1; }
if [[ "$command_name" == "-h" || "$command_name" == "--help" ]]; then usage; exit 0; fi
shift

channel="" actor="" title="" summary="" direction="offer" mechanism="fixed"
quantity="" price="" budget="" increment="" decrement="" delivery="" closes=""
listing="" response="" award="" fulfillment="" amount="" message=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --channel) channel="${2:-}"; shift 2 ;;
    --actor) actor="${2:-}"; shift 2 ;;
    --title) title="${2:-}"; shift 2 ;;
    --summary) summary="${2:-}"; shift 2 ;;
    --direction) direction="${2:-}"; shift 2 ;;
    --mechanism) mechanism="${2:-}"; shift 2 ;;
    --quantity) quantity="${2:-}"; shift 2 ;;
    --price) price="${2:-}"; shift 2 ;;
    --budget) budget="${2:-}"; shift 2 ;;
    --increment) increment="${2:-}"; shift 2 ;;
    --decrement) decrement="${2:-}"; shift 2 ;;
    --delivery-minutes) delivery="${2:-}"; shift 2 ;;
    --closes-at) closes="${2:-}"; shift 2 ;;
    --listing) listing="${2:-}"; shift 2 ;;
    --response) response="${2:-}"; shift 2 ;;
    --award) award="${2:-}"; shift 2 ;;
    --fulfillment) fulfillment="${2:-}"; shift 2 ;;
    --amount) amount="${2:-}"; shift 2 ;;
    --message) message="${2:-}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

listing_json() {
  jq -cn --arg actorName "$actor" --arg direction "$direction" --arg mechanism "$mechanism" \
    --arg title "$title" --arg summary "$summary" --arg quantity "$quantity" --arg price "$price" \
    --arg budget "$budget" --arg increment "$increment" --arg decrement "$decrement" --arg delivery "$delivery" --arg closes "$closes" \
    '{actorName:$actorName,direction:$direction,mechanism:$mechanism,title:$title,summary:$summary,quantity:(if $quantity=="unlimited" then "unlimited" else ($quantity|tonumber) end)} + (if $price!="" then {priceSats:($price|tonumber)} else {} end) + (if $budget!="" then {maxBudgetSats:($budget|tonumber)} else {} end) + (if $increment!="" then {minimumIncrementSats:($increment|tonumber)} else {} end) + (if $decrement!="" then {minimumDecrementSats:($decrement|tonumber)} else {} end) + (if $delivery!="" then {deliveryMinutes:($delivery|tonumber)} else {} end) + (if $closes!="" then {closesAt:($closes|tonumber)} else {} end)'
}

case "$command_name" in
  create)
    need actor "$actor"; need title "$title"; need summary "$summary"; need quantity "$quantity"
    slug=$(tr '[:upper:]' '[:lower:]' <<<"$title" | tr -cs 'a-z0-9' '-' | sed 's/^-//;s/-$//' | cut -c1-48)
    description_direction=$(tr '[:lower:]' '[:upper:]' <<<"${direction:0:1}")${direction:1}
    created=$(buzz channels create --name "market-${slug:-listing}" --type stream --visibility open --description "$description_direction: $summary")
    channel=$(jq -r '.channel_id' <<<"$created")
    terms=$(listing_json)
    contract=$(jq -cn --arg protocol 'buzz-market/v0' --arg channelId "$channel" --argjson listing "$terms" '{protocol:$protocol,type:"contract",channelId:$channelId,version:1,listing:$listing}')
    contract_result=$(channel_write "$contract")
    listing=$(jq -r '.event_id' <<<"$contract_result")
    announcement=$(jq -cn --arg protocol 'buzz-market/v0' --arg channelId "$channel" --arg listingEventId "$listing" --argjson listing "$terms" '{protocol:$protocol,type:"announcement",channelId:$channelId,version:1,listingEventId:$listingEventId,listing:$listing}')
    pulse_result=$(buzz social publish --content "$announcement")
    jq -cn --arg channel_id "$channel" --arg listing_event_id "$listing" --arg announcement_event_id "$(jq -r '.event_id' <<<"$pulse_result")" '{channel_id:$channel_id,listing_event_id:$listing_event_id,announcement_event_id:$announcement_event_id}'
    ;;
  response)
    need channel "$channel"; need listing "$listing"; need actor "$actor"; need quantity "$quantity"; need message "$message"
    payload=$(jq -cn --arg protocol 'buzz-market/v0' --arg channelId "$channel" --arg listingEventId "$listing" --arg actorName "$actor" --arg quantity "$quantity" --arg amount "$amount" --arg message "$message" '{protocol:$protocol,type:"response",channelId:$channelId,listingEventId:$listingEventId,actorName:$actorName,quantity:($quantity|tonumber),message:$message} + (if $amount!="" then {amountSats:($amount|tonumber)} else {} end)')
    channel_write "$payload" ;;
  award)
    need channel "$channel"; need listing "$listing"; need response "$response"; need actor "$actor"; need quantity "$quantity"; need amount "$amount"
    payload=$(jq -cn --arg protocol 'buzz-market/v0' --arg channelId "$channel" --arg listingEventId "$listing" --arg responseEventId "$response" --arg actorName "$actor" --arg quantity "$quantity" --arg amount "$amount" '{protocol:$protocol,type:"award",channelId:$channelId,listingEventId:$listingEventId,responseEventId:$responseEventId,actorName:$actorName,quantity:($quantity|tonumber),amountSats:($amount|tonumber)}')
    channel_write "$payload" ;;
  fulfill)
    need channel "$channel"; need listing "$listing"; need award "$award"; need actor "$actor"; need message "$message"
    payload=$(jq -cn --arg protocol 'buzz-market/v0' --arg channelId "$channel" --arg listingEventId "$listing" --arg awardEventId "$award" --arg actorName "$actor" --arg message "$message" '{protocol:$protocol,type:"fulfillment",channelId:$channelId,listingEventId:$listingEventId,awardEventId:$awardEventId,actorName:$actorName,message:$message}')
    channel_write "$payload" ;;
  settle)
    need channel "$channel"; need listing "$listing"; need award "$award"; need fulfillment "$fulfillment"; need actor "$actor"; need amount "$amount"
    payload=$(jq -cn --arg protocol 'buzz-market/v0' --arg channelId "$channel" --arg listingEventId "$listing" --arg awardEventId "$award" --arg fulfillmentEventId "$fulfillment" --arg actorName "$actor" --arg amount "$amount" '{protocol:$protocol,type:"settlement",channelId:$channelId,listingEventId:$listingEventId,awardEventId:$awardEventId,fulfillmentEventId:$fulfillmentEventId,actorName:$actorName,amountSats:($amount|tonumber)}')
    channel_write "$payload" ;;
  watch)
    need channel "$channel"
    buzz messages get --channel "$channel" --limit 100 | jq '[.[] | . as $event | (try (.content | fromjson) catch null) as $body | select($body.protocol == "buzz-market/v0" and $body.channelId == $channel) | {id,pubkey,created_at,type:$body.type,body:$body}] | sort_by(.created_at, ({contract:0,response:1,award:2,fulfillment:3,settlement:4}[.type] // 99), .id)' --arg channel "$channel"
    ;;
  *) usage; die "unknown command: $command_name" ;;
esac
