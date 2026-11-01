#!/bin/sh
# Dev loop for the .app bundle. Every ad-hoc rebuild changes the code
# signature, which invalidates the Accessibility grant while the System
# Settings checkbox still claims otherwise; this script clears the stale
# entry so a fresh prompt appears, then relaunches. Grant once per rebuild;
# the engine starts the moment the grant lands.
set -e
cd "$(dirname "$0")/.."
if [ "${1:-}" = "build" ]; then
	./scripts/build-app.sh
fi
pkill -x mojito 2>/dev/null || true
sleep 1
tccutil reset Accessibility com.mojito.app || true
open target/app/Mojito.app
