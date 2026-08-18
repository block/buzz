#!/usr/bin/env bash
# =============================================================================
# dev-reset.sh — Tear down everything and recreate a clean environment
# =============================================================================
# Usage: ./scripts/dev-reset.sh
#
# Stops all services, removes development desktop state and ALL local service
# volumes (data is lost!), brings everything back up fresh, and runs migrations.
# Installed Buzz state is preserved.
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log()    { echo -e "${BLUE}[dev-reset]${NC} $*"; }
success(){ echo -e "${GREEN}[dev-reset]${NC} $*"; }
warn()   { echo -e "${YELLOW}[dev-reset]${NC} $*"; }
error()  { echo -e "${RED}[dev-reset]${NC} $*" >&2; }

cd "${REPO_ROOT}"

# ---- Confirm ----------------------------------------------------------------

if [[ "${1:-}" != "--yes" ]]; then
  echo -e "${YELLOW}WARNING: This will DELETE all development data (desktop state, postgres, minio volumes).${NC}"
  echo -e "   Installed Buzz app state and its production keyring are preserved."
  echo -e "   Redis data is ephemeral and always wiped on restart."
  echo ""
  read -r -p "Are you sure? [y/N] " confirm
  case "${confirm}" in
    [yY][eE][sS]|[yY]) ;;
    *)
      log "Aborted."
      exit 0
      ;;
  esac
fi

# ---- Desktop development state ---------------------------------------------

log "Removing desktop development state..."
"${SCRIPT_DIR}/reset-desktop-dev-state.sh"
success "Desktop development state removed (installed Buzz state preserved)"

# ---- Tear down --------------------------------------------------------------

log "Stopping and removing containers + volumes..."
docker compose down -v --remove-orphans 2>/dev/null || true
success "Containers and volumes removed"

# ---- Bring back up ----------------------------------------------------------

log "Recreating environment..."
exec "${SCRIPT_DIR}/dev-setup.sh"
