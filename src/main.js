const invoke = window.__TAURI__?.core?.invoke;

const DEFAULT_DELAY_MS = 500;
const DEFAULT_TEXT_OPACITY = 100;
const DEFAULT_BG_OPACITY = 100;
const DEFAULT_LABEL_SIZE = 12;
const DEFAULT_TEMP_SIZE = 32;
const DELAY_KEY = "stm.refreshDelay";
const TRANSPARENT_KEY = "stm.transparent";
const TEXT_OPACITY_KEY = "stm.textOpacity";
const BG_OPACITY_KEY = "stm.bgOpacity";
const LABEL_SIZE_KEY = "stm.labelSize";
const TEMP_SIZE_KEY = "stm.tempSize";
const AUTO_SIZE_KEY = "stm.autoTextSize";
const PIN_KEY = "stm.alwaysOnTop";
const LEGACY_OPACITY_KEY = "stm.opacity";

const readingsEl = document.querySelector("#readings");
const settingsBtn = document.querySelector("#settings-btn");
const settingsPanel = document.querySelector("#settings-panel");
const settingsForm = document.querySelector("#settings-form");
const delayInput = document.querySelector("#delay-input");
const scrim = document.querySelector("#scrim");
const transparentToggle = document.querySelector("#transparent-toggle");
const opacityControls = document.querySelector("#opacity-controls");
const bgOpacityInput = document.querySelector("#bg-opacity-input");
const bgOpacityValue = document.querySelector("#bg-opacity-value");
const textOpacityInput = document.querySelector("#text-opacity-input");
const textOpacityValue = document.querySelector("#text-opacity-value");
const labelSizeInput = document.querySelector("#label-size-input");
const labelSizeValue = document.querySelector("#label-size-value");
const tempSizeInput = document.querySelector("#temp-size-input");
const tempSizeValue = document.querySelector("#temp-size-value");
const autoSizeToggle = document.querySelector("#auto-size-toggle");
const sizeControls = document.querySelector("#size-controls");
const resetBtn = document.querySelector("#reset-btn");

const SENSORS = [
  { key: "cpuPackage", label: "CPU Package" },
  { key: "pCore0", label: "P-Core 0" },
  { key: "gpu", label: "GPU" },
  { key: "ssd", label: "SSD" },
];

let refreshDelay = loadNumber(DELAY_KEY, DEFAULT_DELAY_MS, 1, 60000);
let transparentOn = localStorage.getItem(TRANSPARENT_KEY) === "1";
let textOpacityPct = loadNumber(
  TEXT_OPACITY_KEY,
  loadNumber(LEGACY_OPACITY_KEY, DEFAULT_TEXT_OPACITY, 20, 100),
  20,
  100
);
let bgOpacityPct = loadNumber(BG_OPACITY_KEY, DEFAULT_BG_OPACITY, 0, 100);
let labelSizePx = loadNumber(LABEL_SIZE_KEY, DEFAULT_LABEL_SIZE, 10, 28);
let tempSizePx = loadNumber(TEMP_SIZE_KEY, DEFAULT_TEMP_SIZE, 18, 72);
let autoTextSize = localStorage.getItem(AUTO_SIZE_KEY) === "1";
let pinnedOnTop = localStorage.getItem(PIN_KEY) === "1";
let pollTimer = null;
let settingsOpen = false;
let scrollHideTimer = null;
let pollInFlight = false;

function loadNumber(key, fallback, min, max) {
  const stored = Number.parseInt(localStorage.getItem(key) ?? "", 10);
  if (!Number.isFinite(stored) || stored < min) {
    return fallback;
  }
  return Math.min(stored, max);
}

function parseDelay(raw) {
  const parsed = Number.parseInt(String(raw).trim(), 10);
  if (!Number.isFinite(parsed) || parsed < 1) {
    return DEFAULT_DELAY_MS;
  }
  return Math.min(parsed, 60000);
}

function appWindow() {
  return window.__TAURI__?.window?.getCurrentWindow?.();
}

function tempColor(temp) {
  const t = Math.min(1, Math.max(0, (temp - 20) / 80));
  if (t < 0.5) {
    return `rgb(${Math.round(255 * t * 2)}, 255, 0)`;
  }
  return `rgb(255, ${Math.round(255 * (1 - (t - 0.5) * 2))}, 0)`;
}

function formatValue(value) {
  if (value == null || Number.isNaN(value)) {
    return { text: "—", color: "#7d8782" };
  }
  return {
    text: value.toFixed(1),
    color: tempColor(value),
  };
}

function renderReadings(reading) {
  readingsEl.innerHTML = SENSORS.map((sensor) => {
    const { text, color } = formatValue(reading?.[sensor.key]);
    return `
      <article class="card" style="--heat:${color}">
        <div class="card-stack">
          <span class="card-label">${sensor.label}</span>
          <div class="card-temp">
            <span class="num">${text}</span>
            <span class="deg">°C</span>
          </div>
        </div>
      </article>
    `;
  }).join("");
  if (autoTextSize) {
    requestAnimationFrame(() => applyAppearance());
  }
}

function updateFooter() {
  delayInput.value = String(refreshDelay);
}

function autoFitTextSize() {
  const card = document.querySelector(".card");
  const monitor = document.querySelector(".monitor");
  let width = 0;
  let height = 0;
  if (card && card.clientWidth > 8 && card.clientHeight > 8) {
    width = card.clientWidth - 32;
    height = card.clientHeight - 32;
  } else if (monitor) {
    width = (monitor.clientWidth - 60) / 2 - 32;
    height = (monitor.clientHeight - 116) / 2 - 32;
  }
  width = Math.max(1, width);
  height = Math.max(1, height);
  const temp = Math.round(
    Math.min(160, Math.max(18, Math.min(height * 0.48, width / 2.8)))
  );
  const label = Math.round(
    Math.min(48, Math.max(10, Math.min(temp * 0.36, width / 8)))
  );
  return { label, temp };
}

async function applyAppearance() {
  document.body.classList.toggle("transparent-mode", transparentOn);
  document.body.classList.toggle("auto-text-size", autoTextSize);
  const bgAlpha = bgOpacityPct / 100;
  const textAlpha = textOpacityPct / 100;
  const fitted = autoTextSize ? autoFitTextSize() : { label: labelSizePx, temp: tempSizePx };
  document.documentElement.style.setProperty("--window-opacity", String(bgAlpha));
  document.documentElement.style.setProperty("--content-opacity", String(textAlpha));
  document.documentElement.style.setProperty("--chrome-opacity", String(bgAlpha));
  document.documentElement.style.setProperty("--label-size", `${fitted.label}px`);
  document.documentElement.style.setProperty("--temp-size", `${fitted.temp}px`);

  transparentToggle.setAttribute("aria-checked", transparentOn ? "true" : "false");
  autoSizeToggle?.setAttribute("aria-checked", autoTextSize ? "true" : "false");
  sizeControls?.classList.toggle("is-disabled", autoTextSize);
  if (labelSizeInput) {
    labelSizeInput.disabled = autoTextSize;
  }
  if (tempSizeInput) {
    tempSizeInput.disabled = autoTextSize;
  }
  bgOpacityInput.value = String(bgOpacityPct);
  textOpacityInput.value = String(textOpacityPct);
  bgOpacityValue.textContent = `${bgOpacityPct}%`;
  textOpacityValue.textContent = `${textOpacityPct}%`;
  labelSizeInput.value = String(autoTextSize ? fitted.label : labelSizePx);
  tempSizeInput.value = String(autoTextSize ? fitted.temp : tempSizePx);
  labelSizeValue.textContent = `${autoTextSize ? fitted.label : labelSizePx}px`;
  tempSizeValue.textContent = `${autoTextSize ? fitted.temp : tempSizePx}px`;

  const win = appWindow();
  try {
    await win?.setBackgroundColor({
      red: 12,
      green: 14,
      blue: 13,
      alpha: bgAlpha,
    });
  } catch {
    // Browser preview has no window API.
  }
  try {
    await win?.setShadow(!transparentOn && bgAlpha >= 0.95);
  } catch {
    // Shadow API is optional.
  }
}

async function pollOnce() {
  if (typeof invoke !== "function") {
    renderReadings({});
    return;
  }
  if (pollInFlight) {
    return;
  }
  pollInFlight = true;
  try {
    renderReadings(await invoke("read_temperatures"));
  } catch {
    renderReadings({});
  } finally {
    pollInFlight = false;
  }
}

function startPolling() {
  if (pollTimer != null) {
    window.clearInterval(pollTimer);
  }
  pollOnce();
  pollTimer = window.setInterval(pollOnce, refreshDelay);
}

function applyDelay(raw) {
  refreshDelay = parseDelay(raw);
  localStorage.setItem(DELAY_KEY, String(refreshDelay));
  updateFooter();
  startPolling();
}

function setTransparent(on) {
  transparentOn = on;
  localStorage.setItem(TRANSPARENT_KEY, on ? "1" : "0");
  applyAppearance();
}

function setTextOpacity(value) {
  const parsed = Number.parseInt(value, 10);
  textOpacityPct = Number.isFinite(parsed)
    ? Math.min(100, Math.max(20, parsed))
    : DEFAULT_TEXT_OPACITY;
  localStorage.setItem(TEXT_OPACITY_KEY, String(textOpacityPct));
  applyAppearance();
}

function setLabelSize(value) {
  const parsed = Number.parseInt(value, 10);
  labelSizePx = Number.isFinite(parsed)
    ? Math.min(28, Math.max(10, parsed))
    : DEFAULT_LABEL_SIZE;
  localStorage.setItem(LABEL_SIZE_KEY, String(labelSizePx));
  applyAppearance();
}

function setTempSize(value) {
  const parsed = Number.parseInt(value, 10);
  tempSizePx = Number.isFinite(parsed)
    ? Math.min(72, Math.max(18, parsed))
    : DEFAULT_TEMP_SIZE;
  localStorage.setItem(TEMP_SIZE_KEY, String(tempSizePx));
  applyAppearance();
}

function setBgOpacity(value) {
  const parsed = Number.parseInt(value, 10);
  bgOpacityPct = Number.isFinite(parsed)
    ? Math.min(100, Math.max(0, parsed))
    : DEFAULT_BG_OPACITY;
  localStorage.setItem(BG_OPACITY_KEY, String(bgOpacityPct));
  applyAppearance();
}

function setAutoTextSize(on) {
  autoTextSize = on;
  localStorage.setItem(AUTO_SIZE_KEY, on ? "1" : "0");
  applyAppearance();
}

function resetSettings() {
  localStorage.removeItem(LEGACY_OPACITY_KEY);
  applyDelay(DEFAULT_DELAY_MS);
  setTransparent(false);
  setBgOpacity(DEFAULT_BG_OPACITY);
  setTextOpacity(DEFAULT_TEXT_OPACITY);
  setLabelSize(DEFAULT_LABEL_SIZE);
  setTempSize(DEFAULT_TEMP_SIZE);
  setAutoTextSize(false);
}

function syncPinButton() {
  const pinBtn = document.querySelector("#pin-btn");
  if (!pinBtn) {
    return;
  }
  pinBtn.classList.toggle("pinned", pinnedOnTop);
  pinBtn.setAttribute("aria-pressed", pinnedOnTop ? "true" : "false");
  pinBtn.setAttribute("aria-label", pinnedOnTop ? "Unpin from top" : "Pin on top");
  pinBtn.setAttribute("title", pinnedOnTop ? "Unpin from top" : "Pin on top");
}

async function setAppFocused(focused) {
  document.body.classList.toggle("app-focused", focused);
}

async function setPinnedOnTop(on) {
  pinnedOnTop = on;
  localStorage.setItem(PIN_KEY, on ? "1" : "0");
  syncPinButton();
  try {
    await appWindow()?.setAlwaysOnTop(on);
  } catch {
    // Browser preview has no window API.
  }
}

function setSettingsOpen(open) {
  settingsOpen = open;
  settingsPanel.classList.toggle("open", open);
  settingsBtn.classList.toggle("open", open);
  document.body.classList.toggle("settings-open", open);
  scrim.classList.toggle("visible", open);
  scrim.hidden = false;
  settingsBtn.hidden = open;
  settingsPanel.setAttribute("aria-hidden", open ? "false" : "true");
  settingsBtn.setAttribute("aria-expanded", open ? "true" : "false");
  settingsBtn.setAttribute("aria-label", open ? "Close settings" : "Open settings");
  if (!open) {
    settingsPanel.classList.remove("is-scrolling");
    if (scrollHideTimer != null) {
      window.clearTimeout(scrollHideTimer);
      scrollHideTimer = null;
    }
  }
}

function toggleSettings() {
  setSettingsOpen(!settingsOpen);
}

async function syncMaximizeButton() {
  const win = appWindow();
  const maxBtn = document.querySelector("#max-btn");
  const icon = document.querySelector("#max-icon");
  if (!win || !maxBtn || !icon) {
    return;
  }
  const maximized = await win.isMaximized();
  maxBtn.setAttribute("aria-label", maximized ? "Restore" : "Maximize");
  maxBtn.setAttribute("title", maximized ? "Restore" : "Maximize");
  icon.innerHTML = maximized
    ? '<rect x="3.5" y="3.5" width="6.5" height="6.5" /><path d="M5 3.5V2.5h6.5V9H10.5" />'
    : '<rect x="1.5" y="1.5" width="9" height="9" rx="0.5" />';
}

window.addEventListener("DOMContentLoaded", () => {
  updateFooter();
  applyAppearance();
  renderReadings({});
  startPolling();

  const win = appWindow();
  document.querySelector("#pin-btn")?.addEventListener("click", () => {
    setPinnedOnTop(!pinnedOnTop);
  });
  document.querySelector("#min-btn")?.addEventListener("click", () => win?.minimize());
  document.querySelector("#max-btn")?.addEventListener("click", async () => {
    await win?.toggleMaximize();
    await syncMaximizeButton();
  });
  document.querySelector("#close-btn")?.addEventListener("click", () => win?.close());
  document.querySelector(".titlebar")?.addEventListener("dblclick", async () => {
    await win?.toggleMaximize();
    await syncMaximizeButton();
  });
  win?.onResized(() => {
    syncMaximizeButton();
    if (autoTextSize) {
      applyAppearance();
    }
  });
  syncMaximizeButton();
  setPinnedOnTop(pinnedOnTop);
  setAppFocused(document.hasFocus());
  win?.isFocused?.()
    .then((focused) => setAppFocused(focused))
    .catch(() => {});
  win?.onFocusChanged?.((event) => {
    setAppFocused(Boolean(event?.payload));
  });
  window.addEventListener("focus", () => setAppFocused(true));
  window.addEventListener("blur", () => setAppFocused(false));
  window.addEventListener("resize", () => {
    if (autoTextSize) {
      applyAppearance();
    }
  });
  const monitorEl = document.querySelector(".monitor");
  if (monitorEl && typeof ResizeObserver !== "undefined") {
    new ResizeObserver(() => {
      if (autoTextSize) {
        applyAppearance();
      }
    }).observe(monitorEl);
  }

  settingsBtn.addEventListener("click", (event) => {
    event.stopPropagation();
    toggleSettings();
  });

  settingsPanel.addEventListener(
    "scroll",
    () => {
      settingsPanel.classList.add("is-scrolling");
      if (scrollHideTimer != null) {
        window.clearTimeout(scrollHideTimer);
      }
      scrollHideTimer = window.setTimeout(() => {
        settingsPanel.classList.remove("is-scrolling");
        scrollHideTimer = null;
      }, 650);
    },
    { passive: true }
  );

  scrim.addEventListener("click", () => setSettingsOpen(false));
  document.addEventListener("pointerdown", (event) => {
    if (!settingsOpen) {
      return;
    }
    if (event.target.closest("#settings-panel")) {
      return;
    }
    if (event.target.closest(".window-controls")) {
      return;
    }
    if (event.target.closest("#settings-btn")) {
      return;
    }
    setSettingsOpen(false);
  });

  settingsForm.addEventListener("submit", (event) => {
    event.preventDefault();
    applyDelay(delayInput.value);
  });

  transparentToggle.addEventListener("click", () => {
    setTransparent(!transparentOn);
  });
  autoSizeToggle.addEventListener("click", () => {
    setAutoTextSize(!autoTextSize);
  });
  bgOpacityInput.addEventListener("input", (event) => {
    setBgOpacity(event.target.value);
  });
  textOpacityInput.addEventListener("input", (event) => {
    setTextOpacity(event.target.value);
  });
  labelSizeInput.addEventListener("input", (event) => {
    setLabelSize(event.target.value);
  });
  tempSizeInput.addEventListener("input", (event) => {
    setTempSize(event.target.value);
  });
  resetBtn.addEventListener("click", () => {
    resetSettings();
  });

  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && settingsOpen) {
      delayInput.value = String(refreshDelay);
      setSettingsOpen(false);
    }
  });
});
