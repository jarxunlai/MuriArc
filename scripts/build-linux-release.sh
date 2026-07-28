#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

die() {
  printf 'build-linux-release: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage:
  build-linux-release.sh --expected-commit <40-hex> \
    --output-root <new-absolute-directory-outside-git> \
    --postgres-source docker.io/library/postgres@sha256:<digest> \
    --tool-root <pinned-cosign-syft-grype-bin-directory> \
    --cosign-key <private-key> --cosign-public-key <public-key> \
    --cosign-password-file <password-file>

This formal command always runs the complete Rust/UI gates, builds and pushes
one digest-pinned MuriArc image, mirrors/signs the pinned PostgreSQL 17 image,
packages deterministic Native and Managed Compose bundles, and creates
descriptor/SBOM/Grype/provenance/Cosign evidence. There is no skip path.
EOF
}

expected_commit=
output_root=
postgres_source=
tool_root=
cosign_key=
cosign_public_key=
cosign_password_file=
while (($#)); do
  case "$1" in
    --expected-commit) expected_commit=${2-}; shift 2 ;;
    --output-root) output_root=${2-}; shift 2 ;;
    --postgres-source) postgres_source=${2-}; shift 2 ;;
    --tool-root) tool_root=${2-}; shift 2 ;;
    --cosign-key) cosign_key=${2-}; shift 2 ;;
    --cosign-public-key) cosign_public_key=${2-}; shift 2 ;;
    --cosign-password-file) cosign_password_file=${2-}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[[ "$expected_commit" =~ ^[0-9a-f]{40}$ ]] ||
  die "--expected-commit must be lowercase 40-hex"
[[ "$output_root" = /* && ! -e "$output_root" && ! -L "$output_root" ]] ||
  die "--output-root must be a new absolute path"
[[ "$postgres_source" =~ ^docker\.io/library/postgres@sha256:[0-9a-f]{64}$ ]] ||
  die "--postgres-source must pin the official PostgreSQL image by digest"
for value in "$tool_root" "$cosign_key" "$cosign_public_key" "$cosign_password_file"; do
  [[ "$value" = /* ]] || die "tool and signing paths must be absolute"
done

repo_root=$(git rev-parse --show-toplevel 2>/dev/null) ||
  die "run from a MuriArc worktree"
cd "$repo_root"
origin=$(git remote get-url origin | sed 's:/*$::')
[[ "$origin" == "https://github.com/jarxunlai/MuriArc.git" ||
   "$origin" == "https://github.com/jarxunlai/MuriArc" ]] ||
  die "origin is not canonical MuriArc"
git fetch --no-tags --prune origin '+refs/heads/main:refs/remotes/origin/main'
[[ "$(git rev-parse HEAD)" == "$expected_commit" ]] ||
  die "HEAD differs from expected commit"
[[ "$(git rev-parse refs/remotes/origin/main)" == "$expected_commit" ]] ||
  die "fresh origin/main differs from expected commit"
[[ -z "$(git status --porcelain=v1 --untracked-files=all)" ]] ||
  die "formal release source must be clean"
python3 - "$repo_root" "$output_root" <<'PY' ||
  die "output root must remain outside Git"
from pathlib import Path
import sys
source = Path(sys.argv[1]).resolve(strict=True)
output = Path(sys.argv[2]).resolve(strict=False)
try:
    output.relative_to(source)
except ValueError:
    raise SystemExit(0)
raise SystemExit(1)
PY

[[ "$(gh api user --jq .login)" == "jarxunlai" ]] ||
  die "GitHub CLI account must be jarxunlai before GHCR writes"

cosign="$tool_root/cosign"
syft="$tool_root/syft"
grype="$tool_root/grype"
for tool in "$cosign" "$syft" "$grype"; do
  [[ -x "$tool" ]] || die "required pinned tool is unavailable: $tool"
done
[[ -f "$cosign_key" && ! -L "$cosign_key" ]] || die "Cosign key is invalid"
[[ -f "$cosign_public_key" && ! -L "$cosign_public_key" ]] ||
  die "Cosign public key is invalid"
[[ -s "$cosign_password_file" && ! -L "$cosign_password_file" ]] ||
  die "Cosign password file is invalid"

"$cosign" version | grep -Eq '(^|[^0-9])v?3\.1\.2([^0-9]|$)' ||
  die "Cosign must be the pinned 3.1.2 release"
"$syft" version | grep -Eq '(^|[^0-9])1\.49\.0([^0-9]|$)' ||
  die "Syft must be the pinned 1.49.0 release"
"$grype" version | grep -Eq '(^|[^0-9])0\.116\.0([^0-9]|$)' ||
  die "Grype must be the pinned 0.116.0 release"

[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
command -v cargo >/dev/null || die "cargo is unavailable"
command -v docker >/dev/null || die "docker is unavailable"
command -v gh >/dev/null || die "gh is unavailable"
command -v corepack >/dev/null || die "corepack is unavailable"
if [[ -d /mnt/e/Muriarc ]]; then
  export CARGO_TARGET_DIR=/mnt/e/Muriarc/builds/cargo-target/shared
else
  export CARGO_TARGET_DIR="$HOME/.cache/muriarc-cargo-target/shared"
fi
mkdir -p "$CARGO_TARGET_DIR"
source_date_epoch=$(git show -s --format=%ct "$expected_commit")
export SOURCE_DATE_EPOCH="$source_date_epoch" TZ=UTC LC_ALL=C
short_commit=${expected_commit:0:12}
invocation_id="muriarc-1.0.0-${short_commit}-$(date -u +%Y%m%dT%H%M%SZ)"

mkdir -p "$output_root"
staging="$output_root/staging"
artifacts="$output_root/artifacts"
evidence="$output_root/evidence"
mkdir -p "$staging" "$artifacts" "$evidence"

corepack enable
corepack prepare pnpm@11.5.0 --activate
pnpm --dir ui install --frozen-lockfile
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
pnpm --dir ui audit --audit-level=high
pnpm --dir ui run test
pnpm --dir ui run typecheck
pnpm --dir ui exec playwright install chromium
pnpm --dir ui run test:e2e
VITE_MURIARC_GATEWAY=remote pnpm --dir ui run build
cargo build --locked --release \
  -p muriarc-server \
  -p muriarc-upgrade-executor \
  -p muriarc-verifier \
  -p muriarc-release-fixture \
  -p muriarcctl \
  --features muriarc-server/postgres,muriarc-release-fixture/postgres

native_root="$staging/native-system"
python3 scripts/build_server_bundle.py \
  --profile native-system \
  --version 1.0.0 \
  --output "$native_root" \
  --server "$CARGO_TARGET_DIR/release/muriarc-server" \
  --controller "$CARGO_TARGET_DIR/release/muriarcctl" \
  --upgrade-executor "$CARGO_TARGET_DIR/release/muriarc-upgrade-executor" \
  --verifier "$CARGO_TARGET_DIR/release/muriarc-verifier" \
  --fixture-producer "$CARGO_TARGET_DIR/release/muriarc-release-fixture" \
  --ui-dir "$repo_root/ui/dist" \
  --deploy-root "$repo_root/deploy"
native_artifact="$artifacts/MuriArc-1.0.0-native-system-linux-amd64-${short_commit}.tar.gz"
python3 scripts/package_release_tree.py \
  --root "$native_root" \
  --output "$native_artifact" \
  --prefix "MuriArc-1.0.0-native-system" \
  --source-date-epoch "$source_date_epoch"

token=$(gh auth token)
printf '%s' "$token" | docker login ghcr.io --username jarxunlai --password-stdin >/dev/null
unset token

server_tag="ghcr.io/jarxunlai/muriarc-server:1.0.0-${short_commit}"
server_metadata="$staging/server-build-metadata.json"
docker buildx build \
  --platform linux/amd64 \
  --file Dockerfile \
  --tag "$server_tag" \
  --provenance=mode=max \
  --sbom=true \
  --push \
  --metadata-file "$server_metadata" \
  "$repo_root"
server_digest=$(python3 - "$server_metadata" <<'PY'
import json,re,sys
value=json.load(open(sys.argv[1], encoding="utf-8")).get("containerimage.digest", "")
if not re.fullmatch(r"sha256:[0-9a-f]{64}", value):
    raise SystemExit("Docker metadata has no immutable image digest")
print(value)
PY
)
server_ref="ghcr.io/jarxunlai/muriarc-server@${server_digest}"

postgres_tag="ghcr.io/jarxunlai/muriarc-postgres:17-${short_commit}"
docker buildx imagetools create --tag "$postgres_tag" "$postgres_source"
postgres_digest=$(docker buildx imagetools inspect "$postgres_tag" | python3 -c '
import re,sys
matches=re.findall(r"(?m)^Digest:\s*(sha256:[0-9a-f]{64})\s*$", sys.stdin.read())
if len(matches) != 1:
    raise SystemExit("cannot resolve mirrored PostgreSQL digest")
print(matches[0])
')
postgres_ref="ghcr.io/jarxunlai/muriarc-postgres@${postgres_digest}"

export COSIGN_PASSWORD
COSIGN_PASSWORD=$(cat "$cosign_password_file")
server_signature="$staging/muriarc-server.cosign.bundle.json"
postgres_signature="$staging/postgres-17.cosign.bundle.json"
"$cosign" sign --yes --key "$cosign_key" --bundle "$server_signature" "$server_ref"
"$cosign" verify --key "$cosign_public_key" --bundle "$server_signature" "$server_ref" >/dev/null
"$cosign" sign --yes --key "$cosign_key" --bundle "$postgres_signature" "$postgres_ref"
"$cosign" verify --key "$cosign_public_key" --bundle "$postgres_signature" "$postgres_ref" >/dev/null
unset COSIGN_PASSWORD

docker pull "$server_ref"
docker pull "$postgres_ref"
server_image_archive="$staging/muriarc-server.docker.tar"
postgres_image_archive="$staging/postgres-17.docker.tar"
docker image save --output "$server_image_archive" "$server_ref"
docker image save --output "$postgres_image_archive" "$postgres_ref"

image_evidence="$staging/image-evidence"
mkdir "$image_evidence"
for pair in "server=$server_ref" "postgres=$postgres_ref"; do
  name=${pair%%=*}
  reference=${pair#*=}
  "$syft" scan "registry:$reference" -o "cyclonedx-json=$image_evidence/$name.sbom.cdx.json"
  "$grype" "sbom:$image_evidence/$name.sbom.cdx.json" \
    -o json --file "$image_evidence/$name.grype.json" --fail-on high
done

image_lock="$staging/image-lock.json"
python3 - "$image_lock" "$expected_commit" "$server_ref" "$postgres_source" "$postgres_ref" \
  "$server_image_archive" "$postgres_image_archive" "$server_signature" "$postgres_signature" <<'PY'
from pathlib import Path
import hashlib,json,sys
def digest(path):
    h=hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024*1024), b""):
            h.update(block)
    return "sha256:"+h.hexdigest()
value={
  "format_version":1,
  "source_commit":sys.argv[2],
  "server_image":sys.argv[3],
  "postgres_source_image":sys.argv[4],
  "postgres_image":sys.argv[5],
  "server_image_archive_digest":digest(sys.argv[6]),
  "postgres_image_archive_digest":digest(sys.argv[7]),
  "server_signature_bundle_digest":digest(sys.argv[8]),
  "postgres_signature_bundle_digest":digest(sys.argv[9]),
}
Path(sys.argv[1]).write_text(json.dumps(value,indent=2,sort_keys=True)+"\n",encoding="utf-8")
PY

managed_root="$staging/managed-compose"
python3 scripts/build_server_bundle.py \
  --profile managed-compose \
  --version 1.0.0 \
  --output "$managed_root" \
  --controller "$CARGO_TARGET_DIR/release/muriarcctl" \
  --upgrade-executor "$CARGO_TARGET_DIR/release/muriarc-upgrade-executor" \
  --verifier "$CARGO_TARGET_DIR/release/muriarc-verifier" \
  --deploy-root "$repo_root/deploy" \
  --server-image-archive "$server_image_archive" \
  --postgres-image-archive "$postgres_image_archive" \
  --image-lock "$image_lock" \
  --server-image-signature "$server_signature" \
  --postgres-image-signature "$postgres_signature" \
  --image-evidence-dir "$image_evidence"
managed_artifact="$artifacts/MuriArc-1.0.0-managed-compose-linux-amd64-${short_commit}.tar.gz"
python3 scripts/package_release_tree.py \
  --root "$managed_root" \
  --output "$managed_artifact" \
  --prefix "MuriArc-1.0.0-managed-compose" \
  --source-date-epoch "$source_date_epoch"

identity_wrapper="$staging/release-identity-wrapper.json"
"$native_root/bin/muriarc-verifier" identity --output json > "$identity_wrapper"
identity="$output_root/release-identity.json"
python3 - "$identity_wrapper" "$identity" <<'PY'
from pathlib import Path
import json,sys
wrapper=json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
if wrapper.get("ok") is not True or not isinstance(wrapper.get("data"),dict):
    raise SystemExit("final verifier did not export release identity")
Path(sys.argv[2]).write_text(json.dumps(wrapper["data"],indent=2)+"\n",encoding="utf-8")
PY

python3 scripts/finalize_release_artifact.py \
  --source-root "$repo_root" \
  --expected-commit "$expected_commit" \
  --artifact-name native-system \
  --artifact "$native_artifact" \
  --media-type application/vnd.muriarc.native.v1+tar+gzip \
  --output-directory "$evidence/native-system" \
  --invocation-id "$invocation_id-native" \
  --cosign "$cosign" \
  --cosign-key "$cosign_key" \
  --cosign-public-key "$cosign_public_key" \
  --cosign-password-file "$cosign_password_file" \
  --syft "$syft" \
  --grype "$grype"

python3 scripts/finalize_release_artifact.py \
  --source-root "$repo_root" \
  --expected-commit "$expected_commit" \
  --artifact-name managed-compose \
  --artifact "$managed_artifact" \
  --media-type application/vnd.muriarc.compose.v1+tar+gzip \
  --output-directory "$evidence/managed-compose" \
  --invocation-id "$invocation_id-compose" \
  --cosign "$cosign" \
  --cosign-key "$cosign_key" \
  --cosign-public-key "$cosign_public_key" \
  --cosign-password-file "$cosign_password_file" \
  --syft "$syft" \
  --grype "$grype"

[[ -z "$(git status --porcelain=v1 --untracked-files=all)" ]] ||
  die "formal build dirtied the source tree"
printf 'MURIARC_LINUX_RELEASE_BUILD=PASS\n'
printf 'release_identity=%s\n' "$identity"
printf 'native_artifact=%s\n' "$native_artifact"
printf 'native_descriptor=%s\n' "$evidence/native-system/descriptor.json"
printf 'managed_artifact=%s\n' "$managed_artifact"
printf 'managed_descriptor=%s\n' "$evidence/managed-compose/descriptor.json"
printf 'server_image=%s\n' "$server_ref"
printf 'postgres_image=%s\n' "$postgres_ref"
