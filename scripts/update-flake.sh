#!/usr/bin/env bash
set -euo pipefail

FLAKE_NIX="flake.nix"
CARGO_TOML="Cargo.toml"

cd "$(git rev-parse --show-toplevel)"

usage() {
  cat <<EOF
Usage: $(basename "$0") [command]

Commands:
  sync-version   Sync version from Cargo.toml into flake.nix
  update-hash    Recalculate cargoHash in flake.nix
  update-lock    Run nix flake update
  check          Run the full verification checklist
  all            Run sync-version + update-hash + update-lock + check

If no command is given, runs 'all'.
EOF
}

sync_version() {
  local cargo_version
  cargo_version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$CARGO_TOML" | head -1)
  if [[ -z "$cargo_version" ]]; then
    echo "ERROR: Could not read version from $CARGO_TOML" >&2
    exit 1
  fi

  local flake_version
  flake_version=$(sed -n 's/.*version = "\(.*\)";/\1/p' "$FLAKE_NIX" | head -1)

  if [[ "$cargo_version" == "$flake_version" ]]; then
    echo "Version already in sync: $cargo_version"
  else
    sed -i "s/version = \"${flake_version}\";/version = \"${cargo_version}\";/" "$FLAKE_NIX"
    echo "Updated flake.nix version: $flake_version -> $cargo_version"
  fi
}

update_hash() {
  # Clear the hash to force a mismatch
  sed -i 's/cargoHash = "sha256-.*";/cargoHash = "";/' "$FLAKE_NIX"

  echo "Building to calculate cargoHash (this may take a while)..."
  local build_output
  build_output=$(nix build --no-link 2>&1) && {
    echo "Build succeeded with empty hash — cargoHash may already be correct."
    return 0
  }

  local new_hash
  new_hash=$(echo "$build_output" | sed -n 's/.*got: *\(sha256-.*\)/\1/p' | head -1)

  if [[ -z "$new_hash" ]]; then
    echo "ERROR: Could not extract hash from build output:" >&2
    echo "$build_output" >&2
    exit 1
  fi

  sed -i "s|cargoHash = \"\";|cargoHash = \"${new_hash}\";|" "$FLAKE_NIX"
  echo "Updated cargoHash: $new_hash"

  echo "Verifying build..."
  nix build --no-link
  echo "Build verified."
}

update_lock() {
  echo "Updating flake.lock..."
  nix flake update
  echo "flake.lock updated."
}

check() {
  echo "Running verification checklist..."

  echo "  nix build --no-link"
  nix build --no-link

  echo "  nix run . -- --version"
  nix run . -- --version

  echo "  nix flake check"
  nix flake check

  echo "  nix develop -c cargo --version"
  nix develop -c cargo --version

  echo "All checks passed."
}

cmd="${1:-all}"

case "$cmd" in
  sync-version) sync_version ;;
  update-hash)  update_hash ;;
  update-lock)  update_lock ;;
  check)        check ;;
  all)
    sync_version
    update_hash
    update_lock
    check
    ;;
  -h|--help|help) usage ;;
  *)
    echo "Unknown command: $cmd" >&2
    usage >&2
    exit 1
    ;;
esac
