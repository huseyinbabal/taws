# Maintaining the Nix Flake

## Quick start

Use `scripts/update-flake.sh` to automate all maintenance tasks:

```bash
# Run everything (sync version, update hash, update lock, verify)
./scripts/update-flake.sh

# Or run individual steps
./scripts/update-flake.sh sync-version   # Sync version from Cargo.toml to flake.nix
./scripts/update-flake.sh update-hash    # Recalculate cargoHash
./scripts/update-flake.sh update-lock    # Update flake.lock (nixpkgs pin)
./scripts/update-flake.sh check          # Run full verification checklist
```

## What each command does

### `sync-version`

Reads the version from `Cargo.toml` and updates it in `flake.nix`. Run this after bumping the crate version.

### `update-hash`

Recalculates `cargoHash` in `flake.nix`. This is needed whenever `Cargo.lock` changes (new/updated dependencies). The script:

1. Clears the hash to trigger a mismatch
2. Runs `nix build` and extracts the correct hash from the error output
3. Writes the new hash back into `flake.nix`
4. Verifies the build succeeds

### `update-lock`

Runs `nix flake update` to pin a newer version of nixpkgs. Do this periodically to pick up newer Rust toolchains and fixes.

### `check`

Runs the full verification checklist:

```bash
nix build --no-link           # Build succeeds
nix run . -- --version        # Binary runs correctly
nix flake check               # Flake structure is valid
nix develop -c cargo --version  # Dev shell works
```

## Manual steps (if needed)

To update `cargoHash` manually:

1. Set `cargoHash = "";` in `flake.nix`
2. Run `nix build --no-link 2>&1`
3. Copy the hash from the `got:` line in the error output
4. Replace the empty string with the new hash
5. Run `nix build --no-link` to verify
