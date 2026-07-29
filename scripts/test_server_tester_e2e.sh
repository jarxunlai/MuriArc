#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'

usage() {
  echo "Usage: $0 --bundle-dir DIR --mode empty|demo [--cleanup]" >&2
}

BUNDLE_DIR=''
MODE=''
CLEANUP=0
SKIP_VERIFY=0
while (($#)); do
  case "$1" in
    --bundle-dir) BUNDLE_DIR=${2-}; shift 2 ;;
    --mode) MODE=${2-}; shift 2 ;;
    --cleanup) CLEANUP=1; shift ;;
    --skip-verify) SKIP_VERIFY=1; shift ;;
    *) usage; exit 2 ;;
  esac
done
[[ -d "$BUNDLE_DIR" && ( "$MODE" == empty || "$MODE" == demo ) ]] || { usage; exit 2; }
BUNDLE_DIR=$(cd -- "$BUNDLE_DIR" && pwd -P)
CONTROL="$BUNDLE_DIR/muriarc-tester.sh"
[[ -x "$CONTROL" ]] || { echo "missing executable control script" >&2; exit 1; }

short=$(python3 - <<'PY'
import secrets
print(secrets.token_hex(5))
PY
)
project="muriarc-e2e-${MODE}-${short}"
port=$(python3 - <<'PY'
import socket
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
)
password=$(python3 - <<'PY'
import secrets
print(secrets.token_hex(24))
PY
)
root_password=$(python3 - <<'PY'
import secrets
print(secrets.token_urlsafe(36))
PY
)
root_email="root-${short}@example.invalid"
cp "$BUNDLE_DIR/.env.${MODE}.example" "$BUNDLE_DIR/.env"
python3 - "$BUNDLE_DIR/.env" "$project" "$port" "$password" "$root_email" "$root_password" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text()
replacements = {
    "MURIARC_COMPOSE_PROJECT_NAME=muriarc-server-tester\n": f"MURIARC_COMPOSE_PROJECT_NAME={sys.argv[2]}\n",
    "MURIARC_COMPOSE_PROJECT_NAME=muriarc-server-tester-demo\n": f"MURIARC_COMPOSE_PROJECT_NAME={sys.argv[2]}\n",
    "MURIARC_TESTER_SERVER_PORT=8787": f"MURIARC_TESTER_SERVER_PORT={sys.argv[3]}",
    "REPLACE_WITH_32_PLUS_HEX_CHARACTERS": sys.argv[4],
    "REPLACE_WITH_LAB_DISPLAY_NAME": "MuriArc Tester E2E Lab",
    "REPLACE_WITH_ROOT_EMAIL": sys.argv[5],
    "REPLACE_WITH_ROOT_DISPLAY_NAME": "MuriArc Tester E2E Root",
    "REPLACE_WITH_LONG_UNIQUE_ROOT_PASSWORD": sys.argv[6],
}
for old, new in replacements.items():
    text = text.replace(old, new)
active = "\n".join(line for line in text.splitlines() if not line.lstrip().startswith("#"))
if "REPLACE_" in active or "@@" in active:
    raise SystemExit("unrendered E2E environment placeholder")
path.write_text(text)
path.chmod(0o600)
PY

compose=(docker compose --env-file "$BUNDLE_DIR/.env" --project-name "$project" --file "$BUNDLE_DIR/compose.yaml")
cleanup() {
  local rc=$?
  if ((CLEANUP)); then
    "${compose[@]}" down --volumes --remove-orphans >/dev/null 2>&1 || true
  fi
  rm -f "$BUNDLE_DIR/.env" "$BUNDLE_DIR/.cookies" "$BUNDLE_DIR/.login.json" "$BUNDLE_DIR/.login-payload.json" "$BUNDLE_DIR/.data.json" "$BUNDLE_DIR/.second-init.log"
  exit "$rc"
}
trap cleanup EXIT

if ((SKIP_VERIFY == 0)); then "$CONTROL" verify; fi
"$CONTROL" "init-${MODE}"
"$CONTROL" status

if "$CONTROL" "init-${MODE}" >"$BUNDLE_DIR/.second-init.log" 2>&1; then
  echo "second initialization unexpectedly succeeded" >&2
  exit 1
fi
grep -q 'volume already exists' "$BUNDLE_DIR/.second-init.log"

base="http://127.0.0.1:${port}"
python3 - "$root_email" "$root_password" >"$BUNDLE_DIR/.login-payload.json" <<'PY'
import json, sys
print(json.dumps({"email": sys.argv[1], "password": sys.argv[2]}))
PY
curl --noproxy '*' --fail --silent --show-error \
  --cookie-jar "$BUNDLE_DIR/.cookies" \
  --header 'Content-Type: application/json' \
  --data-binary "@$BUNDLE_DIR/.login-payload.json" \
  "$base/api/v1/auth/login" > "$BUNDLE_DIR/.login.json"
rm -f "$BUNDLE_DIR/.login-payload.json"
python3 - "$BUNDLE_DIR/.login.json" "$root_email" <<'PY'
import json, sys
value = json.load(open(sys.argv[1], encoding="utf-8"))
user = value.get("data", {}).get("user", {})
if user.get("email") != sys.argv[2]:
    raise SystemExit("Root login response did not contain the expected user")
PY

if [[ "$MODE" == demo ]]; then
  curl --noproxy '*' --fail --silent --show-error --cookie "$BUNDLE_DIR/.cookies" \
    "$base/api/v1/projects" > "$BUNDLE_DIR/.data.json"
  curl --noproxy '*' --fail --silent --show-error --cookie "$BUNDLE_DIR/.cookies" \
    "$base/api/v1/animals" >> "$BUNDLE_DIR/.data.json"
  python3 - "$BUNDLE_DIR/.data.json" <<'PY'
from pathlib import Path
text = Path(__import__('sys').argv[1]).read_text(encoding="utf-8")
if "standard-v1" not in text and "STD-M-001" not in text:
    raise SystemExit("Root could not observe the standard-v1 project/animal data")
PY
fi

"$CONTROL" down
docker volume inspect "${project}_postgres_data" >/dev/null
docker volume inspect "${project}_server_data" >/dev/null
"$CONTROL" up
curl --noproxy '*' --fail --silent --show-error "$base/readyz" >/dev/null
curl --noproxy '*' --fail --silent --show-error \
  --header 'Content-Type: application/json' \
  --data "$(python3 -c 'import json,sys; print(json.dumps({"email":sys.argv[1],"password":sys.argv[2]}))' "$root_email" "$root_password")" \
  "$base/api/v1/auth/login" >/dev/null
printf 'PASS: Server Tester %s E2E, reinitialization refusal, and volume persistence.\n' "$MODE"
