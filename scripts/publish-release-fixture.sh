#!/usr/bin/env bash
set -Eeuo pipefail

die() {
  printf 'publish-release-fixture: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage:
  publish-release-fixture.sh --fixture <dir> --repository <ghcr.io/repo/name> \
    --tag <immutable-tag> --manifest-digest <sha256:...>

The script validates an already generated Fixture, pushes a deterministic tar
as an OCI Artifact, signs the digest-pinned reference, and prints JSON. It never
generates historical data and never edits release-fixtures/catalog.json.
EOF
}

fixture=
repository=
tag=
manifest_digest=
while (($#)); do
  case "$1" in
    --fixture) fixture=${2-}; shift 2 ;;
    --repository) repository=${2-}; shift 2 ;;
    --tag) tag=${2-}; shift 2 ;;
    --manifest-digest) manifest_digest=${2-}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[[ -d "$fixture" ]] || die "--fixture must be an existing directory"
[[ "$repository" == ghcr.io/* && "$repository" != *@* && "$repository" != *:* ]] ||
  die "--repository must be an untagged ghcr.io repository"
[[ "$tag" =~ ^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$ && "$tag" != latest ]] ||
  die "--tag must be an immutable non-latest OCI tag"
[[ "$manifest_digest" =~ ^sha256:[0-9a-f]{64}$ ]] ||
  die "--manifest-digest must be lowercase SHA-256"

for tool in cargo oras cosign tar python3; do
  command -v "$tool" >/dev/null || die "required tool is missing: $tool"
done

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) ||
  die "run this script from a MuriArc worktree"
cd "$repo_root"

if [[ -n "${MURIARC_VERIFIER:-}" ]]; then
  [[ -x "$MURIARC_VERIFIER" ]] || die "MURIARC_VERIFIER is not executable"
  verifier=("$MURIARC_VERIFIER")
else
  verifier=(cargo run --locked --quiet -p muriarc-verifier --)
fi
"${verifier[@]}" asset --root "$fixture" --manifest-digest "$manifest_digest" --output json >/dev/null

temporary=$(mktemp -d "${TMPDIR:-/tmp}/muriarc-fixture-publish.XXXXXX")
trap 'rm -rf "$temporary"' EXIT
archive="$temporary/fixture.tar"
tar --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 --numeric-owner \
  --format=posix -cf "$archive" -C "$fixture" .

tag_reference="${repository}:${tag}"
oras push "$tag_reference" \
  --artifact-type application/vnd.muriarc.release-fixture.v1 \
  "$archive:application/vnd.muriarc.release-fixture.layer.v1+tar" >/dev/null
oci_digest=$(oras resolve "$tag_reference")
[[ "$oci_digest" =~ ^sha256:[0-9a-f]{64}$ ]] ||
  die "registry did not return a valid OCI manifest digest"
pinned_reference="${repository}@${oci_digest}"

if [[ -n "${COSIGN_KEY:-}" ]]; then
  [[ -n "${COSIGN_PUBLIC_KEY:-}" ]] ||
    die "COSIGN_PUBLIC_KEY is required when COSIGN_KEY is used"
  cosign sign --yes --key "$COSIGN_KEY" "$pinned_reference" >/dev/null
  cosign verify --key "$COSIGN_PUBLIC_KEY" "$pinned_reference" >/dev/null
else
  identity_regexp=${COSIGN_CERTIFICATE_IDENTITY_REGEXP:-'^https://github.com/jarxunlai/MuriArc/.github/workflows/'}
  oidc_issuer=${COSIGN_CERTIFICATE_OIDC_ISSUER:-'https://token.actions.githubusercontent.com'}
  cosign sign --yes "$pinned_reference" >/dev/null
  cosign verify \
    --certificate-identity-regexp "$identity_regexp" \
    --certificate-oidc-issuer "$oidc_issuer" \
    "$pinned_reference" >/dev/null
fi

archive_digest=$(python3 - "$archive" <<'PY'
import hashlib
import pathlib
import sys

hasher = hashlib.sha256()
with pathlib.Path(sys.argv[1]).open("rb") as stream:
    for block in iter(lambda: stream.read(1024 * 1024), b""):
        hasher.update(block)
print("sha256:" + hasher.hexdigest())
PY
)
python3 - "$pinned_reference" "$oci_digest" "$archive_digest" "$manifest_digest" <<'PY'
import json
import sys

print(json.dumps({
    "oci_reference": sys.argv[1],
    "fixture_artifact_digest": sys.argv[2],
    "fixture_tar_digest": sys.argv[3],
    "fixture_manifest_digest": sys.argv[4],
}, separators=(",", ":")))
PY
