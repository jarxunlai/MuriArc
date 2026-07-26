#!/usr/bin/env bash
set -Eeuo pipefail

die() {
  printf 'pull-release-fixture: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage:
  pull-release-fixture.sh --reference <ghcr.io/...@sha256:...> \
    --output <external-cache-dir> --manifest-digest <sha256:...>
EOF
}

reference=
output=
manifest_digest=
while (($#)); do
  case "$1" in
    --reference) reference=${2-}; shift 2 ;;
    --output) output=${2-}; shift 2 ;;
    --manifest-digest) manifest_digest=${2-}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[[ "$reference" =~ ^ghcr\.io/.+@sha256:[0-9a-f]{64}$ && "$reference" != *:latest* ]] ||
  die "--reference must be a digest-pinned GHCR OCI reference"
[[ -n "$output" ]] || die "--output is required"
[[ "$manifest_digest" =~ ^sha256:[0-9a-f]{64}$ ]] ||
  die "--manifest-digest must be lowercase SHA-256"
for tool in oras cosign python3 git; do
  command -v "$tool" >/dev/null || die "required tool is missing: $tool"
done

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) ||
  die "run this script from a MuriArc worktree"
if python3 - "$repo_root" "$output" <<'PY'
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
  die "Fixture caches must remain outside the Git worktree"
fi

cd "$repo_root"
if [[ -n "${MURIARC_VERIFIER:-}" ]]; then
  [[ -x "$MURIARC_VERIFIER" ]] || die "MURIARC_VERIFIER is not executable"
  verifier=("$MURIARC_VERIFIER")
else
  command -v cargo >/dev/null || die "cargo is required when MURIARC_VERIFIER is not set"
  verifier=(cargo run --locked --quiet -p muriarc-verifier --)
fi

if [[ -d "$output" ]]; then
  "${verifier[@]}" asset --root "$output" --manifest-digest "$manifest_digest" --output json >/dev/null
  printf 'verified cached Fixture: %s\n' "$output"
  exit 0
fi
[[ ! -e "$output" ]] || die "--output exists but is not a directory"

if [[ -n "${COSIGN_PUBLIC_KEY:-}" ]]; then
  cosign verify --key "$COSIGN_PUBLIC_KEY" "$reference" >/dev/null
else
  identity_regexp=${COSIGN_CERTIFICATE_IDENTITY_REGEXP:-'^https://github.com/jarxunlai/MuriArc/.github/workflows/'}
  oidc_issuer=${COSIGN_CERTIFICATE_OIDC_ISSUER:-'https://token.actions.githubusercontent.com'}
  cosign verify \
    --certificate-identity-regexp "$identity_regexp" \
    --certificate-oidc-issuer "$oidc_issuer" \
    "$reference" >/dev/null
fi

parent=$(dirname "$output")
mkdir -p "$parent"
temporary=$(mktemp -d "$parent/.muriarc-fixture-pull.XXXXXX")
trap 'rm -rf "$temporary"' EXIT
mkdir "$temporary/oci" "$temporary/extracted"
oras pull "$reference" --output "$temporary/oci" >/dev/null

mapfile -t archives < <(find "$temporary/oci" -type f -name fixture.tar -print)
((${#archives[@]} == 1)) || die "OCI Artifact must contain exactly one fixture.tar"
if find "$temporary/oci" ! -type d ! -type f -print -quit | grep -q .; then
  die "OCI Artifact contains a symlink or special file"
fi

python3 - "${archives[0]}" "$temporary/extracted" <<'PY'
from pathlib import Path, PurePosixPath
import shutil
import sys
import tarfile

archive = Path(sys.argv[1])
destination = Path(sys.argv[2])
with tarfile.open(archive, "r:") as bundle:
    members = bundle.getmembers()
    for member in members:
        path = PurePosixPath(member.name)
        if path.is_absolute() or ".." in path.parts:
            raise SystemExit(f"unsafe archive path: {member.name}")
        if not (member.isdir() or member.isfile()):
            raise SystemExit(f"archive contains link or special file: {member.name}")
    for member in members:
        relative = PurePosixPath(member.name)
        if str(relative) in {".", ""}:
            continue
        target = destination.joinpath(*relative.parts)
        if member.isdir():
            target.mkdir(parents=True, exist_ok=True)
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        source = bundle.extractfile(member)
        if source is None:
            raise SystemExit(f"cannot read archive member: {member.name}")
        with source, target.open("xb") as stream:
            shutil.copyfileobj(source, stream)
PY

"${verifier[@]}" asset --root "$temporary/extracted" \
  --manifest-digest "$manifest_digest" --output json >/dev/null
mv "$temporary/extracted" "$output"
printf 'verified Fixture restored to %s\n' "$output"
