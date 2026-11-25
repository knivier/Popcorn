#!/usr/bin/env bash
# Wi-Fi firmware for the boot stick (Qualcomm WCN6855, see scripts/firmware/).
#
# The firmware is Qualcomm's code for the processors inside the Wi-Fi chip; it is
# not stored in this repository. `core.sh firmware` (and `core.sh all`, when files
# are missing) downloads it from linux-firmware into target/firmware/wcn6855/ and
# keeps a file only if its SHA-256 and size match the pinned manifest. The image
# build copies verified files to /FW/WCN6855/ with a MANIFEST.TXT the kernel
# re-checks before loading. Without network or with POPCORN_FIRMWARE=off the
# image still builds, just without Wi-Fi firmware (`wifi` then says so).

POPCORN_FW_MANIFEST="${POPCORN_FW_MANIFEST:-$POPCORN_SCRIPTS/firmware/wcn6855.manifest}"
POPCORN_FW_DIR="${POPCORN_FW_DIR:-${POPCORN_TARGET:-$POPCORN_SCRIPTS/../target}/firmware/wcn6855}"
POPCORN_FW_URL="${POPCORN_FW_URL:-https://git.kernel.org/pub/scm/linux/kernel/git/firmware/linux-firmware.git/plain}"

firmware_sha256() {
  if have sha256sum; then
    sha256sum "$1" | cut -d' ' -f1
  elif have shasum; then
    shasum -a 256 "$1" | cut -d' ' -f1
  else
    echo "no-sha256-tool"
  fi
}

# Manifest rows without comments: "sha256 bytes source-path stick-name".
firmware_rows() {
  local line
  while IFS= read -r line || [[ -n "$line" ]]; do
    line="${line%%#*}"
    [[ -z "${line// /}" ]] && continue
    printf '%s\n' "$line"
  done <"$POPCORN_FW_MANIFEST"
}

# 0 if $1 has SHA-256 $2 and size $3.
firmware_file_ok() {
  local f="$1" sha="$2" size="$3"
  [[ -f "$f" ]] || return 1
  [[ "$(wc -c <"$f" | tr -d ' ')" == "$size" ]] || return 1
  [[ "$(firmware_sha256 "$f")" == "$sha" ]] || return 1
}

# 0 when every manifest file is present and verified.
firmware_all_ok() {
  local sha size src name
  [[ -f "$POPCORN_FW_MANIFEST" ]] || return 1
  while read -r sha size src name; do
    firmware_file_ok "$POPCORN_FW_DIR/$name" "$sha" "$size" || return 1
  done < <(firmware_rows)
}

# Download what is missing or wrong. A file is kept only once verified.
firmware_fetch() {
  local sha size src name dst missing=0
  [[ -f "$POPCORN_FW_MANIFEST" ]] || { log WARN "firmware manifest missing: $POPCORN_FW_MANIFEST"; return 1; }
  mkdir -p "$POPCORN_FW_DIR"
  while read -r sha size src name; do
    dst="$POPCORN_FW_DIR/$name"
    if firmware_file_ok "$dst" "$sha" "$size"; then
      continue
    fi
    rm -f "$dst" "$dst.part"
    if ! have curl; then
      log WARN "firmware: curl not installed; cannot fetch $src"
      missing=1
      continue
    fi
    log INFO "firmware: fetching $src ($size bytes)"
    if ! curl -fsSL --retry 2 --max-time 300 -o "$dst.part" "$POPCORN_FW_URL/$src"; then
      log WARN "firmware: download failed: $src"
      rm -f "$dst.part"
      missing=1
      continue
    fi
    if firmware_file_ok "$dst.part" "$sha" "$size"; then
      mv "$dst.part" "$dst"
    else
      log WARN "firmware: $src does not match the pinned SHA-256/size; discarded"
      rm -f "$dst.part"
      missing=1
    fi
  done < <(firmware_rows)
  if [[ $missing -eq 0 ]]; then
    log SUCCESS "firmware: WCN6855 files verified in $POPCORN_FW_DIR"
    return 0
  fi
  return 1
}

# Copy verified firmware into the image staging tree (FW/WCN6855/ + MANIFEST.TXT).
# Never copies an unverified file; on any doubt the stick simply has no firmware.
firmware_stage() {
  local stage="$1" sha size src name out
  if [[ "${POPCORN_FIRMWARE:-auto}" == off ]]; then
    log INFO "firmware: POPCORN_FIRMWARE=off, image has no Wi-Fi firmware"
    return 0
  fi
  if ! firmware_all_ok; then
    firmware_fetch >&2 || true
  fi
  if ! firmware_all_ok; then
    log WARN "firmware: WCN6855 files missing or unverified; image has no Wi-Fi firmware (run: core.sh firmware)"
    return 0
  fi
  out="$stage/FW/WCN6855"
  mkdir -p "$out"
  {
    printf '# Popcorn Wi-Fi firmware manifest: sha256 bytes name\n'
    while read -r sha size src name; do
      cp "$POPCORN_FW_DIR/$name" "$out/$name"
      printf '%s %s %s\n' "$sha" "$size" "$name"
    done < <(firmware_rows)
  } >"$out/MANIFEST.TXT"
  log INFO "firmware: WCN6855 files staged to /FW/WCN6855"
}
