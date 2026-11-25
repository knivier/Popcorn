#!/usr/bin/env bash
# Grant lint (driver framework, virtio-net review §2 "Enforcement").
# New driver code reaches hardware only through grants (drivers/grant): no raw
# port I/O, no direct-map pointer games, no ad-hoc MMIO mapping, no `static mut`.
# Usage: ./scripts/core.sh lint   (exit 1 on any hit)

lint_grants() {
  local root="${POPCORN_SRC:?}/rust/popcorn_kernel/src"
  local dirs=("$root/drivers/virtio" "$root/drivers/net" "$root/drivers/wifi" "$root/net")
  local pattern='asm!|drivers::io|grant::port::raw|\binl\(|\boutl\(|\binb\(|\boutb\(|\binw\(|\boutw\(|virt_to_phys|phys_to_virt|map_mmio|static mut'
  local hits=0 d
  for d in "${dirs[@]}"; do
    [[ -d "$d" ]] || continue
    # Comment lines (// and //!) may name the forbidden APIs.
    if grep -rnE "$pattern" "$d" --include='*.rs' | grep -vE ':[0-9]+:[[:space:]]*//'; then
      hits=1
    fi
  done
  # Crate-wide: no new `static mut` anywhere (all 69 were converted in M0).
  if grep -rnE '^\s*(pub(\([a-z:]+\))?\s+)?static mut ' "$root" --include='*.rs'; then
    hits=1
  fi
  if [[ $hits -ne 0 ]]; then
    log ERROR "lint: raw hardware access or static mut found (see lines above)"
    return 1
  fi
  log SUCCESS "lint: grant rules clean (virtio/, net/, wifi/; no static mut in the crate)"
}
