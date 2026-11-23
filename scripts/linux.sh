#!/usr/bin/env bash
# Generic Linux entry (Debian/Ubuntu/Arch/...): CLI → core.sh; bare run → dialog.
# The shared body lives in lib/wrapper.sh.
set -Eeuo pipefail
POPCORN_PLATFORM_LABEL="Linux"
# shellcheck source=lib/wrapper.sh
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/wrapper.sh"
