#!/usr/bin/env bash
# macOS entry: CLI delegates to core.sh; bare run opens dialog menu.
# The shared body lives in lib/wrapper.sh.
set -Eeuo pipefail
POPCORN_PLATFORM_LABEL="macOS"
# shellcheck source=lib/wrapper.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/wrapper.sh"
