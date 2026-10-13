#!/bin/sh
# Runs locally what CI runs on every pull request (except the Android emulator).
# Usage: scripts/check.sh            # all areas
#        scripts/check.sh core app   # selected areas: core, app, server
set -eu

root=$(git rev-parse --show-toplevel)
areas=${*:-core app server}

step() { printf '\n\033[1;36m▸ %s\033[0m\n' "$1"; }

for area in $areas; do
  case "$area" in
    core)
      cd "$root"
      step "core: rustfmt";  cargo fmt --all -- --check
      step "core: clippy";   cargo clippy --workspace --all-targets --locked -- -D warnings
      step "core: tests";    cargo test --workspace --locked
      step "core: deny";     cargo deny check
      ;;
    app)
      cd "$root/app"
      step "app: format";    dart format --output=none --set-exit-if-changed lib test integration_test test_driver hook
      step "app: analyze";   flutter analyze --fatal-infos
      step "app: tests";     flutter test
      step "app: windows integration test"
      flutter test integration_test/app_test.dart -d windows
      ;;
    server)
      cd "$root/server"
      step "server: check";  npm run check
      ;;
    *)
      echo "unknown area: $area (expected core, app, server)" >&2
      exit 2
      ;;
  esac
done

printf '\n\033[1;32m✔ all checks passed (%s)\033[0m\n' "$areas"
