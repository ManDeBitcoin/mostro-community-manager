#!/usr/bin/env bash
# Verifies an official mostrod release before it is pinned:
#   - its manifest.txt carries at least one valid signature by a maintainer
#     whose key is kept in config/upstream/keys/ (primary fingerprints in
#     FINGERPRINTS); keys or signatures from unknown signers are not trusted;
#   - both musl binaries match their sha256 lines in that manifest.
# Downloads into OUTDIR and prints KEY=VALUE lines for the bump:
#   amd64_sha256=…  arm64_sha256=…  signers=negrunch,arkanoider
#
# Usage: scripts/verify-mostrod-release.sh VERSION OUTDIR
set -euo pipefail
version="${1:?usage: verify-mostrod-release.sh VERSION OUTDIR}"
out="${2:?usage: verify-mostrod-release.sh VERSION OUTDIR}"
cd "$(dirname "$0")/.."
keys="$PWD/config/upstream/keys"
base="https://github.com/MostroP2P/mostro/releases/download/v$version"
mkdir -p "$out"

fail() { echo "verify-mostrod-release: $*" >&2; exit 1; }

curl -fsSL "https://api.github.com/repos/MostroP2P/mostro/releases/tags/v$version" -o "$out/release.json" \
  || fail "release v$version not found"
python3 -I -c 'import json, sys; [print(a["name"]) for a in json.load(open(sys.argv[1]))["assets"]]' "$out/release.json" > "$out/assets"
curl -fsSL "$base/manifest.txt" -o "$out/manifest.txt" || fail "no manifest.txt in v$version"

gnupg="$(mktemp -d)"
trap 'rm -rf "$gnupg"' EXIT
chmod 700 "$gnupg"
for key in "$keys"/*.asc; do
  GNUPGHOME="$gnupg" gpg --batch --quiet --import "$key" 2>/dev/null
done

signers=""
while IFS= read -r asset; do
  case "$asset" in manifest.txt.sig.*) ;; *) continue ;; esac
  curl -fsSL "$base/$asset" -o "$out/$asset"
  # VALIDSIG <signing key> … <primary key>: trust is decided on the primary key.
  primary="$(GNUPGHOME="$gnupg" gpg --batch --status-fd 1 --verify "$out/$asset" "$out/manifest.txt" 2>/dev/null \
    | awk '$2 == "VALIDSIG" { print $NF }' || true)"
  name="$(awk -v fpr="$primary" '!/^#/ && $1 == fpr { print $2 }' "$keys/FINGERPRINTS")"
  if [ -n "$primary" ] && [ -n "$name" ]; then
    echo "valid signature: $name ($primary)" >&2
    signers="${signers:+$signers,}$name"
  else
    echo "not trusted: $asset" >&2
  fi
done < "$out/assets"
[ -n "$signers" ] || fail "v$version has no valid signature by a known maintainer"

checksum() { # checksum ARCH-TRIPLE: verified sha256 of that binary
  local file="mostrod-$1"
  curl -fsSL "$base/$file" -o "$out/$file" || fail "no $file in v$version"
  local expected actual
  expected="$(awk -v f="$file" '{ n = split($2, p, "/") } p[n] == f { print $1; exit }' "$out/manifest.txt")"
  actual="$(sha256sum "$out/$file" | cut -d' ' -f1)"
  [ -n "$expected" ] || fail "$file is not in the manifest"
  [ "$expected" = "$actual" ] || fail "$file does not match its manifest checksum"
  echo "$actual"
}
amd64="$(checksum x86_64-unknown-linux-musl)"
arm64="$(checksum aarch64-unknown-linux-musl)"

echo "amd64_sha256=$amd64"
echo "arm64_sha256=$arm64"
echo "signers=$signers"
