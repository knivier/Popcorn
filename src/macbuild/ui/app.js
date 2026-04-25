let lastIdx = -1;
let autoscroll = true;

const elTerm = document.getElementById("term");
const elDot = document.getElementById("statusDot");
const elStatus = document.getElementById("statusLabel");
const elPill = document.getElementById("pillRunning");
const elToolchain = document.getElementById("toolchainLabel");
const elMkrescue = document.getElementById("mkrescueLabel");

const btnRecomp = document.getElementById("btnRecomp");
const btnBuild = document.getElementById("btnBuild");
const btnIso = document.getElementById("btnIso");
const btnRun = document.getElementById("btnRun");
const btnStopQemu = document.getElementById("btnStopQemu");
const btnClear = document.getElementById("btnClear");

function setStatus(kind, text) {
  elStatus.textContent = text;
  if (kind === "ok") {
    elDot.style.background = "var(--ok)";
    elDot.style.boxShadow = "0 0 0 4px rgba(67,245,154,.12), 0 0 22px rgba(67,245,154,.35)";
  } else if (kind === "warn") {
    elDot.style.background = "var(--warn)";
    elDot.style.boxShadow = "0 0 0 4px rgba(255,211,107,.12), 0 0 22px rgba(255,211,107,.25)";
  } else if (kind === "err") {
    elDot.style.background = "var(--err)";
    elDot.style.boxShadow = "0 0 0 4px rgba(255,90,95,.12), 0 0 22px rgba(255,90,95,.25)";
  } else {
    elDot.style.background = "rgba(255,255,255,.5)";
    elDot.style.boxShadow = "0 0 0 4px rgba(255,255,255,.08)";
  }
}

function appendLine(ev) {
  const div = document.createElement("div");
  div.className = `line lvl-${ev.level}`;
  div.textContent = `[${ev.level}] ${ev.message}`;
  elTerm.appendChild(div);
}

function refreshButtons(state) {
  const busy = state.busy;
  btnRecomp.disabled = busy;
  btnBuild.disabled = busy;
  btnIso.disabled = busy;
  btnRun.disabled = busy;
  btnClear.disabled = busy;
  btnStopQemu.disabled = !state.qemu_running;
  elPill.textContent = busy ? "Running" : "Idle";
}

async function poll() {
  try {
    const state = await window.pywebview.api.get_state();
    elToolchain.textContent = state.toolchain || "Missing";
    elMkrescue.textContent = state.mkrescue || "Missing";
    refreshButtons(state);

    if (state.last_level === "ERROR") setStatus("err", state.status || "Error");
    else if (state.last_level === "WARN") setStatus("warn", state.status || "Warning");
    else setStatus("ok", state.status || "Ready");

    const events = await window.pywebview.api.poll_logs(lastIdx);
    for (const ev of events) {
      lastIdx = Math.max(lastIdx, ev.idx);
      appendLine(ev);
    }
    if (autoscroll && events.length) {
      // More reliable than scrollTop=scrollHeight with animated inserts.
      requestAnimationFrame(() => {
        elTerm.lastElementChild?.scrollIntoView({ block: "end" });
      });
    }
  } catch (e) {
    setStatus("warn", "Disconnected");
  } finally {
    setTimeout(poll, 120);
  }
}

btnClear.addEventListener("click", async () => {
  elTerm.innerHTML = "";
  lastIdx = -1;
  await window.pywebview.api.clear_logs();
});

btnBuild.addEventListener("click", async () => {
  await window.pywebview.api.run_action("build");
});
btnIso.addEventListener("click", async () => {
  await window.pywebview.api.run_action("iso");
});
btnRun.addEventListener("click", async () => {
  await window.pywebview.api.run_action("run");
});
btnRecomp.addEventListener("click", async () => {
  await window.pywebview.api.run_action("recomp");
});
btnStopQemu.addEventListener("click", async () => {
  await window.pywebview.api.stop_qemu();
});

// Keep autoscroll enabled unless user scrolls away from bottom.
elTerm.addEventListener("scroll", () => {
  const nearBottom = elTerm.scrollTop + elTerm.clientHeight >= elTerm.scrollHeight - 24;
  autoscroll = nearBottom;
});

poll();

