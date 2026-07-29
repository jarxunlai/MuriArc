#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
readonly SCRIPT_DIR
readonly COMPOSE_FILE="$SCRIPT_DIR/compose.yaml"
readonly BOOTSTRAP_FILE="$SCRIPT_DIR/compose.bootstrap.yaml"
ENV_FILE="${MURIARC_TESTER_ENV_FILE:-$SCRIPT_DIR/.env}"

usage() {
  cat <<'TXT'
Usage: ./muriarc-tester.sh <verify|init-empty|init-demo|up|status|logs|down> [--env-file PATH]

  verify     verify bundle integrity, configuration, Docker and pinned images
  init-empty initialize a brand-new empty deployment, then disable bootstrap
  init-demo  initialize brand-new volumes with synthetic standard-v1 data
  up         start an already initialized deployment
  status     show Compose status and query /readyz
  logs       show recent redacted-safe container logs (never paste secrets)
  down       stop containers while preserving all named volumes
TXT
}

die() { printf 'ERROR: %s\n' "$*" >&2; exit 1; }
warn() { printf 'WARNING: %s\n' "$*" >&2; }

while (($# > 0)); do
  case "$1" in
    verify|init-empty|init-demo|up|status|logs|down)
      [[ -z "${COMMAND:-}" ]] || die "only one command may be supplied"
      COMMAND=$1
      shift
      ;;
    --env-file)
      (($# >= 2)) || die "--env-file requires a path"
      ENV_FILE=$2
      shift 2
      ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done
[[ -n "${COMMAND:-}" ]] || { usage; exit 2; }

require_command() { command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"; }

env_value() {
  local key=$1 count value
  [[ -f "$ENV_FILE" && ! -L "$ENV_FILE" ]] || die "missing regular environment file: $ENV_FILE"
  count=$(grep -cE "^${key}=" "$ENV_FILE" || true)
  [[ "$count" == 1 ]] || die "$key must appear exactly once in $ENV_FILE"
  value=$(sed -nE "s/^${key}=//p" "$ENV_FILE")
  value=${value%$'\r'}
  [[ -n "$value" ]] || die "$key must not be empty"
  printf '%s' "$value"
}

validate_environment() {
  local -a required
  local key ttl
  required=(
    MURIARC_TESTER_DATASET_MODE MURIARC_COMPOSE_PROJECT_NAME
    MURIARC_TESTER_SOURCE_COMMIT MURIARC_TESTER_SERVER_PORT
    MURIARC_POSTGRES_DB MURIARC_POSTGRES_USER MURIARC_POSTGRES_PASSWORD
    MURIARC_DATA_ROOT MURIARC_ATTACHMENT_ROOT MURIARC_AI_MASTER_KEY_FILE
    MURIARC_LAB_ID MURIARC_LAB_NAME MURIARC_ROOT_USER_ID
    MURIARC_ROOT_USER_EMAIL MURIARC_ROOT_USER_NAME MURIARC_ROOT_PASSWORD
    MURIARC_SESSION_COOKIE_SECURE MURIARC_SESSION_TTL_HOURS
  )
  for key in "${required[@]}"; do env_value "$key" >/dev/null; done
  grep -Eq '(^|=)(REPLACE_|@@|<[^>]+>)' "$ENV_FILE" \
    && die "replace every placeholder in $ENV_FILE before continuing"
  grep -Eq '^MURIARC_(AI_MASTER_KEY|BOOTSTRAP_TOKEN|BOOTSTRAP_MCP_TOKEN)=' "$ENV_FILE" \
    && die "shared AI/bootstrap secrets are forbidden in the Tester environment file"

  PROJECT=$(env_value MURIARC_COMPOSE_PROJECT_NAME)
  MODE=$(env_value MURIARC_TESTER_DATASET_MODE)
  PORT=$(env_value MURIARC_TESTER_SERVER_PORT)
  SOURCE_COMMIT=$(env_value MURIARC_TESTER_SOURCE_COMMIT)
  [[ "$PROJECT" =~ ^[a-z0-9][a-z0-9_-]{2,62}$ ]] || die "invalid Compose project name"
  [[ "$MODE" == empty || "$MODE" == demo ]] || die "dataset mode must be empty or demo"
  [[ "$PORT" =~ ^[0-9]+$ ]] || die "Tester Server port must be an integer"
  ((PORT >= 1024 && PORT <= 65535)) || die "Tester Server port must be 1024..65535"
  [[ "$SOURCE_COMMIT" =~ ^[0-9a-f]{40}$ ]] || die "Tester source commit must be 40 lowercase hex characters"
  [[ "$(env_value MURIARC_LAB_ID)" != "$(env_value MURIARC_ROOT_USER_ID)" ]] \
    || die "Lab ID and Root user ID must differ"
  [[ "$(env_value MURIARC_POSTGRES_PASSWORD)" =~ ^[A-Za-z0-9_-]{32,}$ ]] \
    || die "PostgreSQL password must be at least 32 URL-safe characters"
  [[ "$(env_value MURIARC_ROOT_PASSWORD)" =~ ^[A-Za-z0-9_-]{32,}$ ]] \
    || die "Root password must be at least 32 URL-safe characters"
  ttl=$(env_value MURIARC_SESSION_TTL_HOURS)
  [[ "$ttl" =~ ^[0-9]+$ ]] || die "Session TTL must be an integer"
  ((ttl >= 1 && ttl <= 720)) || die "Session TTL must be 1..720 hours"
  case "$(env_value MURIARC_SESSION_COOKIE_SECURE)" in true|false) ;; *) die "cookie secure must be true or false" ;; esac
  if [[ "$(uname -s)" != Linux* ]]; then
    warn "This image is linux/amd64; use Docker Desktop Linux containers on Windows."
  fi
}

compose() {
  docker compose --env-file "$ENV_FILE" --project-name "$PROJECT" --file "$COMPOSE_FILE" "$@"
}

compose_bootstrap() {
  docker compose --env-file "$ENV_FILE" --project-name "$PROJECT" \
    --file "$COMPOSE_FILE" --file "$BOOTSTRAP_FILE" "$@"
}

volume_exists() { docker volume inspect "$1" >/dev/null 2>&1; }

assert_fresh() {
  local volume
  for volume in "${PROJECT}_postgres_data" "${PROJECT}_server_data"; do
    ! volume_exists "$volume" || die "refusing initialization: volume already exists: $volume"
  done
  [[ -z "$(compose ps --all --quiet 2>/dev/null || true)" ]] \
    || die "refusing initialization: Compose resources already exist for $PROJECT"
}

assert_initialized() {
  volume_exists "${PROJECT}_postgres_data" || die "deployment is not initialized; run init-empty or init-demo"
  volume_exists "${PROJECT}_server_data" || die "deployment is not initialized; run init-empty or init-demo"
}

ready() {
  local url="http://127.0.0.1:${PORT}/readyz" i
  for ((i = 1; i <= 60; i++)); do
    if curl --noproxy '*' --fail --silent --show-error "$url" >/dev/null 2>&1; then
      printf 'Ready: %s\n' "$url"
      return 0
    fi
    sleep 2
  done
  compose ps >&2 || true
  die "Server did not become ready at $url"
}

verify_checksums() {
  local checksums="$SCRIPT_DIR/CHECKSUMS.sha256" line digest file actual
  [[ -f "$checksums" && ! -L "$checksums" ]] || die "CHECKSUMS.sha256 is missing"
  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ -n "$line" ]] || continue
    digest=${line%%  *}; file=${line#*  }
    [[ "$digest" =~ ^[0-9a-f]{64}$ ]] || die "invalid checksum entry"
    [[ "$file" != /* && "$file" != *'..'* && "$file" != *\\* ]] || die "unsafe checksum path: $file"
    [[ -f "$SCRIPT_DIR/$file" && ! -L "$SCRIPT_DIR/$file" ]] || die "missing checked file: $file"
    actual=$(sha256sum "$SCRIPT_DIR/$file" | awk '{print $1}')
    [[ "$actual" == "$digest" ]] || die "checksum mismatch: $file"
  done < "$checksums"
}

verify_bundle() {
  require_command docker; require_command curl; require_command sha256sum
  docker info >/dev/null 2>&1 || die "Docker Engine is not available"
  docker compose version >/dev/null 2>&1 || die "Docker Compose v2 is required"
  validate_environment
  verify_checksums
  grep -Eq 'ghcr\.io/jarxunlai/muriarc-server-tester@sha256:[0-9a-f]{64}' "$COMPOSE_FILE" \
    || die "Server image is not pinned to the expected GHCR digest"
  grep -Eq 'postgres:17-bookworm@sha256:[0-9a-f]{64}' "$COMPOSE_FILE" \
    || die "PostgreSQL image is not pinned to an immutable digest"
  ! grep -Eqi '(:latest|5432:5432|/var/run/docker\.sock|0\.0\.0\.0:[^ ]*:8787)' "$COMPOSE_FILE" \
    || die "Compose template violates the Tester network/image boundary"
  compose config --quiet
  local image
  while IFS= read -r image; do
    docker buildx imagetools inspect "$image" >/dev/null \
      || die "cannot resolve pinned image anonymously/currently: $image"
  done < <(compose config --images | sort -u)
  printf 'PASS: bundle, environment, Compose policy and pinned images verified.\n'
}

init_empty() {
  validate_environment
  [[ "$MODE" == empty ]] || die "init-empty requires .env.empty.example as the configuration base"
  assert_fresh
  trap 'printf "Initialization failed; containers and volumes are preserved for diagnosis. Do not clear or SQL-patch them.\n" >&2' ERR
  compose_bootstrap up --detach --wait --wait-timeout 240 db server
  ready
  compose_bootstrap stop server
  compose up --detach --wait --wait-timeout 240 server
  ready
  trap - ERR
  printf 'Empty Tester initialized. Bootstrap is now disabled; data volumes are retained.\n'
}

init_demo() {
  validate_environment
  [[ "$MODE" == demo ]] || die "init-demo requires .env.demo.example as the configuration base"
  [[ "$(env_value MURIARC_LAB_ID)" == 4d555249-4152-4300-0000-000000000001 ]] \
    || die "demo Lab UUID must remain the standard-v1 fixed UUID"
  [[ "$(env_value MURIARC_ROOT_USER_ID)" == 4d555249-4152-4300-0000-000000000002 ]] \
    || die "demo Root UUID must remain the standard-v1 fixed user UUID"
  assert_fresh
  trap 'printf "Demo initialization failed; containers and volumes are preserved for diagnosis. Do not clear or SQL-patch them.\n" >&2' ERR
  compose up --detach --wait --wait-timeout 180 db
  compose --profile demo-tools run --rm seed-standard-v1
  compose --profile demo-tools run --rm seed-standard-v1 \
    verify-postgres --fixture /opt/muriarc/fixtures/standard-v1 \
    --output /var/lib/muriarc/generation --source-commit "$SOURCE_COMMIT"
  compose --profile demo-tools run --rm --entrypoint /bin/sh seed-standard-v1 \
    -ec 'install -m 0600 /var/lib/muriarc/generation/deployment-generation.json /var/lib/muriarc/generation/data/deployment-generation.json'
  compose up --detach --wait --wait-timeout 240 server
  ready
  trap - ERR
  printf 'Synthetic standard-v1 Tester initialized and verified. Bootstrap remains disabled.\n'
}

start_existing() { validate_environment; assert_initialized; compose up --detach --wait --wait-timeout 240 db server; ready; }
show_status() { validate_environment; compose ps; ready; }
show_logs() { validate_environment; compose logs --no-color --tail 200 db server; }
stop_preserving() { validate_environment; compose down --remove-orphans; printf 'Stopped; named volumes were preserved.\n'; }

case "$COMMAND" in
  verify) verify_bundle ;;
  init-empty) require_command docker; require_command curl; validate_environment; init_empty ;;
  init-demo) require_command docker; require_command curl; validate_environment; init_demo ;;
  up) require_command docker; require_command curl; start_existing ;;
  status) require_command docker; require_command curl; show_status ;;
  logs) require_command docker; show_logs ;;
  down) require_command docker; stop_preserving ;;
esac
