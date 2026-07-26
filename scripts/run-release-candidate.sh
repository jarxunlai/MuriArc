#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

die() {
  printf 'run-release-candidate: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage:
  run-release-candidate.sh --release-manifest <final-manifest.json> \
    --artifact-lock <signed-artifact-lock.json> \
    --run-root <new-external-directory> \
    [--catalog-baseline release-fixtures/catalog.json] \
    [--matrix-definition release-fixtures/matrix.json] \
    [--rc-definition release-fixtures/rc-gate.json]

Required environment:
  MURIARC_VERIFIER              final standalone verifier binary
  MURIARC_FIXTURE_PRODUCER      real final-artifact Fixture producer/publisher
  MURIARC_COMPATIBILITY_DRIVER real Native/Compose/Desktop compatibility driver
  MURIARC_RC_DRIVER            real systemd/Docker/Windows/Cloudflare RC driver
  MURIARC_FIXTURE_CACHE        external immutable Fixture cache

The Fixture producer must run the exact final artifacts and publish a candidate
append-only Catalog before the matrix starts. The RC driver must execute every
scenario in rc-gate.json. This script never converts missing evidence into SKIP
and never generates evidence itself.
EOF
}

release_manifest=
artifact_lock=
run_root=
catalog_baseline=release-fixtures/catalog.json
matrix_definition=release-fixtures/matrix.json
rc_definition=release-fixtures/rc-gate.json
while (($#)); do
  case "$1" in
    --release-manifest) release_manifest=${2-}; shift 2 ;;
    --artifact-lock) artifact_lock=${2-}; shift 2 ;;
    --run-root) run_root=${2-}; shift 2 ;;
    --catalog-baseline) catalog_baseline=${2-}; shift 2 ;;
    --matrix-definition) matrix_definition=${2-}; shift 2 ;;
    --rc-definition) rc_definition=${2-}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) ||
  die "run this script from a MuriArc worktree"
cd "$repo_root"

[[ -n "$release_manifest" && -f "$release_manifest" && ! -L "$release_manifest" ]] ||
  die "--release-manifest must be a regular non-symlink final manifest"
[[ -n "$artifact_lock" && -f "$artifact_lock" && ! -L "$artifact_lock" ]] ||
  die "--artifact-lock must be a regular non-symlink signed artifact lock"
[[ -n "$run_root" ]] || die "--run-root is required"
[[ ! -e "$run_root" && ! -L "$run_root" ]] ||
  die "--run-root must be a new path so evidence cannot mix with a previous RC"
[[ -f "$catalog_baseline" && ! -L "$catalog_baseline" ]] ||
  die "Fixture Catalog baseline is missing"
[[ -f "$matrix_definition" && ! -L "$matrix_definition" ]] || die "matrix definition is missing"
[[ -f "$rc_definition" && ! -L "$rc_definition" ]] || die "RC gate definition is missing"

verifier=${MURIARC_VERIFIER:-}
fixture_producer=${MURIARC_FIXTURE_PRODUCER:-}
compatibility_driver=${MURIARC_COMPATIBILITY_DRIVER:-}
rc_driver=${MURIARC_RC_DRIVER:-}
fixture_cache=${MURIARC_FIXTURE_CACHE:-}
[[ -x "$verifier" ]] || die "MURIARC_VERIFIER must name the final executable verifier"
[[ -x "$fixture_producer" ]] ||
  die "MURIARC_FIXTURE_PRODUCER must name an executable final-artifact producer"
[[ -x "$compatibility_driver" ]] ||
  die "MURIARC_COMPATIBILITY_DRIVER must name an executable real profile driver"
[[ -x "$rc_driver" ]] || die "MURIARC_RC_DRIVER must name an executable real RC driver"
[[ -n "$fixture_cache" ]] || die "MURIARC_FIXTURE_CACHE is required"

python3 - "$repo_root" "$release_manifest" "$artifact_lock" "$run_root" "$fixture_cache" <<'PY' ||
  die "Release Manifest, artifact lock, run root, and Fixture cache must remain outside the Git worktree"
from pathlib import Path
import sys

root = Path(sys.argv[1]).resolve()
for value in sys.argv[2:]:
    if not value:
        raise SystemExit(1)
    candidate = Path(value).resolve()
    try:
        candidate.relative_to(root)
    except ValueError:
        continue
    raise SystemExit(1)
PY

mkdir -p "$run_root"
candidate_catalog="$run_root/candidate-catalog.json"
"$fixture_producer" \
  --release-manifest "$release_manifest" \
  --artifact-lock "$artifact_lock" \
  --catalog-baseline "$catalog_baseline" \
  --fixture-cache "$fixture_cache" \
  --output "$candidate_catalog"
[[ -f "$candidate_catalog" && ! -L "$candidate_catalog" ]] ||
  die "Fixture producer did not produce a regular candidate Catalog"
python3 scripts/check_fixture_catalog.py \
  --catalog "$candidate_catalog" \
  --previous "$catalog_baseline" \
  --require-non-empty

compatibility_root="$run_root/compatibility"
export MURIARC_TARGET_ARTIFACTS="$release_manifest"
scripts/run-release-compatibility.sh \
  --mode rc \
  --catalog "$candidate_catalog" \
  --definition "$matrix_definition" \
  --run-root "$compatibility_root"

matrix_report="$compatibility_root/matrix-report.json"
[[ -f "$matrix_report" && ! -L "$matrix_report" ]] ||
  die "compatibility matrix did not produce a regular final report"

scenario_root="$run_root/scenarios"
mkdir -p "$scenario_root"
rc_evidence="$run_root/rc-evidence.json"
"$rc_driver" \
  --release-manifest "$release_manifest" \
  --artifact-lock "$artifact_lock" \
  --definition "$rc_definition" \
  --matrix-report "$matrix_report" \
  --run-root "$scenario_root" \
  --output "$rc_evidence"
[[ -f "$rc_evidence" && ! -L "$rc_evidence" ]] ||
  die "RC driver did not produce a regular evidence index"

readiness_report="$run_root/release-readiness-report.json"
python3 scripts/check_release_readiness.py \
  --source-root "$repo_root" \
  --release-manifest "$release_manifest" \
  --artifact-lock "$artifact_lock" \
  --catalog "$candidate_catalog" \
  --catalog-baseline "$catalog_baseline" \
  --matrix-definition "$matrix_definition" \
  --matrix-report "$matrix_report" \
  --rc-definition "$rc_definition" \
  --rc-evidence "$rc_evidence" \
  --output "$readiness_report"

printf 'MuriArc RC PASS: %s\n' "$readiness_report"
