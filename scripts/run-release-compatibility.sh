#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

die() {
  printf 'run-release-compatibility: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage:
  run-release-compatibility.sh --mode pr|nightly|rc
    [--catalog release-fixtures/catalog.json]
    [--definition release-fixtures/matrix.json]
    [--changed-files <file>]
    [--run-root <external-directory>]

For a non-empty plan the following are mandatory:
  MURIARC_VERIFIER              executable final verifier binary
  MURIARC_COMPATIBILITY_DRIVER executable profile driver
  MURIARC_TARGET_ARTIFACTS      signed/digest-pinned target artifact manifest
  MURIARC_FIXTURE_CACHE         external Fixture cache root
EOF
}

mode=
catalog=release-fixtures/catalog.json
definition=release-fixtures/matrix.json
changed_files=
run_root=
while (($#)); do
  case "$1" in
    --mode) mode=${2-}; shift 2 ;;
    --catalog) catalog=${2-}; shift 2 ;;
    --definition) definition=${2-}; shift 2 ;;
    --changed-files) changed_files=${2-}; shift 2 ;;
    --run-root) run_root=${2-}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done
[[ "$mode" == pr || "$mode" == nightly || "$mode" == rc ]] ||
  die "--mode must be pr, nightly, or rc"

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) ||
  die "run this script from a MuriArc worktree"
cd "$repo_root"
[[ -f "$catalog" && -f "$definition" ]] || die "Catalog or matrix definition is missing"

if [[ -z "$run_root" ]]; then
  run_root=$(mktemp -d "${TMPDIR:-/tmp}/muriarc-compatibility.XXXXXX")
else
  mkdir -p "$run_root"
fi
if python3 - "$repo_root" "$run_root" <<'PY'
from pathlib import Path
import sys

root = Path(sys.argv[1]).resolve()
output = Path(sys.argv[2]).resolve()
try:
    output.relative_to(root)
except ValueError:
    raise SystemExit(1)
raise SystemExit(0)
PY
then
  die "compatibility reports and restored data must remain outside the Git worktree"
fi

plan="$run_root/plan.json"
matrix_report="$run_root/matrix-report.json"
report_directory="$run_root/reports"
mkdir -p "$report_directory"
plan_args=(
  python3 scripts/compatibility_matrix.py plan
  --mode "$mode"
  --catalog "$catalog"
  --definition "$definition"
  --output "$plan"
)
if [[ -n "$changed_files" ]]; then
  [[ -f "$changed_files" ]] || die "--changed-files does not exist"
  plan_args+=(--changed-files-file "$changed_files")
fi
"${plan_args[@]}"

run_count=$(python3 - "$plan" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    print(len(json.load(stream)["runs"]))
PY
)
if ((run_count == 0)); then
  [[ "$mode" != rc ]] || die "RC cannot pass without stable Fixtures"
  printf 'No stable Fixture exists yet; %s validates contracts only (0.1 preview scope).\n' "$mode"
  exit 0
fi

verifier=${MURIARC_VERIFIER:-}
driver=${MURIARC_COMPATIBILITY_DRIVER:-}
target_artifacts=${MURIARC_TARGET_ARTIFACTS:-}
fixture_cache=${MURIARC_FIXTURE_CACHE:-}
[[ -x "$verifier" ]] || die "MURIARC_VERIFIER must name an executable binary"
[[ -x "$driver" ]] || die "MURIARC_COMPATIBILITY_DRIVER must name an executable real profile driver"
[[ -f "$target_artifacts" && ! -L "$target_artifacts" ]] ||
  die "MURIARC_TARGET_ARTIFACTS must be a regular signed/digest-pinned manifest"
[[ -n "$fixture_cache" ]] || die "MURIARC_FIXTURE_CACHE is required"
mkdir -p "$fixture_cache"

while IFS=$'\t' read -r fixture_id profile backend reference manifest_digest; do
  [[ -n "$fixture_id" && -n "$profile" && -n "$backend" && -n "$reference" && -n "$manifest_digest" ]] ||
    die "matrix planner produced an incomplete run"
  fixture_root="$fixture_cache/$fixture_id"
  MURIARC_VERIFIER="$verifier" scripts/pull-release-fixture.sh \
    --reference "$reference" \
    --output "$fixture_root" \
    --manifest-digest "$manifest_digest"
  report="$report_directory/${fixture_id}--${profile}.json"
  "$driver" \
    --mode "$mode" \
    --fixture-id "$fixture_id" \
    --fixture-manifest-digest "$manifest_digest" \
    --fixture-root "$fixture_root" \
    --profile "$profile" \
    --target-artifacts "$target_artifacts" \
    --report "$report"
  [[ -f "$report" && ! -L "$report" ]] || die "Driver did not produce a regular report: $report"
  python3 - "$report" "$target_artifacts" "$profile" "$backend" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    report = json.load(stream)
with open(sys.argv[2], encoding="utf-8") as stream:
    release = json.load(stream)
profile = sys.argv[3]
backend = sys.argv[4]
if release.get("format_version") != 1:
    raise SystemExit("target Release Manifest format is invalid")
artifact = release.get("artifacts", {}).get(profile)
if not isinstance(artifact, dict) or artifact.get("digest") != report.get("target_artifact_digest"):
    raise SystemExit("report target artifact is not the profile digest in Release Manifest")
identity = report.get("target_identity", {})
if (
    identity.get("application_version") != release.get("application_version")
    or identity.get("data_epoch") != release.get("data_epoch")
    or identity.get("gateway_contract_revision") != release.get("gateway_contract_revision")
    or identity.get("backend_state_digest") != release.get("backend_states", {}).get(backend)
):
    raise SystemExit("report target identity differs from the signed Release Manifest")
PY
  "$verifier" report --report "$report" --output json >/dev/null
done < <(
  python3 - "$plan" "$catalog" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    plan = json.load(stream)
with open(sys.argv[2], encoding="utf-8") as stream:
    catalog = json.load(stream)
entries = {entry["fixture_id"]: entry for entry in catalog["entries"]}
for run in plan["runs"]:
    entry = entries[run["fixture_id"]]
    print("\t".join((
        run["fixture_id"],
        run["profile"],
        entry["backend"],
        entry["oci_reference"],
        entry["fixture_manifest_digest"],
    )))
PY
)

python3 scripts/compatibility_matrix.py collect \
  --plan "$plan" \
  --report-directory "$report_directory" \
  --output "$matrix_report"
"$verifier" matrix \
  --report "$matrix_report" \
  --definition "$definition" \
  --catalog "$catalog" \
  --output json >/dev/null
printf 'Compatibility matrix PASS: %s\n' "$matrix_report"
