#!/bin/sh
# Rebuild and restart Mojito.app in one step: build-app.sh, then relaunch.
# With the stable "Mojito Dev" identity the Accessibility grant survives
# the rebuild, so the restart is silent; an ad-hoc signature (no identity)
# invalidates the grant, so the stale TCC entry is cleared first, exactly
# as dev-restart.sh does.
set -eu
cd "$(dirname "$0")/.."

./scripts/build-app.sh

pkill -x mojito 2>/dev/null || true
sleep 1

IDENTITY="Mojito Dev"
if ! security find-identity -v -p codesigning 2>/dev/null | grep -q "$IDENTITY"; then
	tccutil reset Accessibility com.mojito.app || true
fi

open target/app/Mojito.app
