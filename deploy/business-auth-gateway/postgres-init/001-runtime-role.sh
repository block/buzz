#!/bin/sh
set -eu

case "$BUSINESS_AUTH_RUNTIME_DATABASE_ROLE" in
  ''|*[!A-Za-z0-9_]*)
    echo "BUSINESS_AUTH_RUNTIME_DATABASE_ROLE must contain only ASCII letters, digits, and underscore" >&2
    exit 1
    ;;
esac

psql \
  --set=ON_ERROR_STOP=1 \
  --set=runtime_role="$BUSINESS_AUTH_RUNTIME_DATABASE_ROLE" \
  --set=runtime_password="$BUSINESS_AUTH_RUNTIME_DATABASE_PASSWORD" \
  --username "$POSTGRES_USER" \
  --dbname "$POSTGRES_DB" <<'SQL'
CREATE ROLE :"runtime_role" LOGIN PASSWORD :'runtime_password' NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT;
SQL
