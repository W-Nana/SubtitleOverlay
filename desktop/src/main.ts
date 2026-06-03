import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import {
  getCurrentWindow,
  PhysicalPosition,
  PhysicalSize,
} from "@tauri-apps/api/window";
import "./style.css";

type SubtitleEntry = {
  timestamp: string;
  original: string;
  translated: string;
};

type OverlayBounds = {
  x: number | null;
  y: number | null;
  width: number;
  height: number;
};

type Settings = {
  serverUrl: string;
  originalTextColor: string;
  translatedTextColor: string;
  backgroundColor: string;
  backgroundOpacity: number;
  fontSize: number;
  fontFamily: string;
  maxSubtitleCount: number;
  overlayBounds: OverlayBounds;
};

type RuntimeState = {
  running: boolean;
  connectionState: ConnectionState;
  subtitles: SubtitleEntry[];
  logs: string[];
  lastError: string | null;
};

type ConnectionState =
  | "disconnected"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "error";

type TauriWindow = ReturnType<typeof getCurrentWindow>;
type ResizeDirection = Parameters<TauriWindow["startResizeDragging"]>[0];

const isTauriRuntime = "__TAURI_INTERNALS__" in window;
const appWindow = isTauriRuntime ? getCurrentWindow() : null;
const appWebview = isTauriRuntime ? getCurrentWebview() : null;
const windowLabel =
  appWindow?.label ?? new URLSearchParams(window.location.search).get("window") ?? "settings";
const viewMode = windowLabel === "overlay" ? "overlay" : "settings";

let settings: Settings = {
  serverUrl: "http://192.168.1.100:8765",
  originalTextColor: "#81D4FA",
  translatedTextColor: "#FFD54F",
  backgroundColor: "#000000",
  backgroundOpacity: 70,
  fontSize: 16,
  fontFamily: "Sans",
  maxSubtitleCount: 3,
  overlayBounds: { x: null, y: null, width: 920, height: 260 },
};

let runtime: RuntimeState = {
  running: false,
  connectionState: "disconnected",
  subtitles: [],
  logs: [],
  lastError: null,
};

const sampleSubtitles: SubtitleEntry[] = [
  { timestamp: "", original: "こんにちは、皆さん", translated: "大家好" },
  { timestamp: "", original: "今日はいい天気ですね", translated: "今天天氣真好呢" },
  { timestamp: "", original: "よろしくお願いします", translated: "請多多指教" },
];

const fallbackFontFamilies = ["Sans", "Serif", "Monospace"];

const appRoot = query<HTMLDivElement>("#app");

let availableFontFamilies = [...fallbackFontFamilies];
let renderUi = () => {};

void boot();

async function boot() {
  if (isTauriRuntime) {
    settings = await invoke<Settings>("get_settings");
    runtime = await invoke<RuntimeState>("get_runtime_state");
    availableFontFamilies = await loadSystemFontFamilies();
  } else {
    runtime.logs = [`[preview] Rendering ${viewMode} window without Tauri runtime.`];
  }

  if (viewMode === "overlay") {
    await setupOverlayWindow();
  } else {
    setupSettingsWindow();
  }

  renderUi();

  if (!isTauriRuntime) {
    return;
  }

  await listen<Settings>("settings-updated", (event) => {
    settings = event.payload;
    renderUi();
  });

  await listen<ConnectionState>("connection-state", (event) => {
    runtime.connectionState = event.payload;
    runtime.running = event.payload !== "disconnected";
    renderUi();
  });

  await listen<SubtitleEntry[]>("subtitle-buffer", (event) => {
    runtime.subtitles = event.payload;
    renderUi();
  });

  await listen<string>("stream-error", (event) => {
    runtime.lastError = event.payload;
    renderUi();
  });

  await listen<string>("log-line", (event) => {
    runtime.logs.push(event.payload);
    runtime.logs = runtime.logs.slice(-200);
    renderUi();
  });
}

function setupSettingsWindow() {
  appRoot.innerHTML = `
    <main class="settings-shell">
      <header class="settings-header">
        <div>
          <h1>SubtitleOverlay</h1>
          <p class="window-subtitle">Desktop Settings</p>
        </div>
        <span class="status-pill" id="statusPill">
          <span class="status-dot"></span>
          <span id="statusText">未連線</span>
        </span>
        <button class="primary-button" id="toggleStream" type="button">啟動</button>
        <button class="secondary-button danger" id="exitButton" type="button">退出</button>
      </header>

      <section class="settings-section connection-section">
        <div class="field full">
          <label for="serverUrl">伺服器位址</label>
          <input id="serverUrl" type="text" spellcheck="false" placeholder="http://192.168.1.100:8765" />
        </div>
      </section>

      <section class="settings-grid">
        <div class="settings-section">
          <h2>外觀</h2>
          <div class="control-grid">
            <div class="field">
              <label for="originalColor">原文顏色</label>
              <input id="originalColor" type="color" />
            </div>
            <div class="field">
              <label for="translatedColor">翻譯顏色</label>
              <input id="translatedColor" type="color" />
            </div>
            <div class="field">
              <label for="backgroundColor">背景顏色</label>
              <input id="backgroundColor" type="color" />
            </div>
            <div class="field">
              <label for="opacity">背景透明度 <span id="opacityValue"></span></label>
              <input id="opacity" type="range" min="0" max="100" step="1" />
            </div>
            <div class="field">
              <label for="fontSize">字體大小 <span id="fontSizeValue"></span></label>
              <input id="fontSize" type="range" min="10" max="32" step="1" />
            </div>
            <div class="field">
              <label for="fontFamily">字體</label>
              <input id="fontFamily" type="text" spellcheck="false" list="fontFamilyOptions" placeholder="Sans" />
              <datalist id="fontFamilyOptions"></datalist>
            </div>
            <div class="field">
              <label for="subtitleCount">保留組數 <span id="subtitleCountValue"></span></label>
              <input id="subtitleCount" type="range" min="1" max="10" step="1" />
            </div>
          </div>
        </div>

        <div class="settings-section">
          <h2>預覽</h2>
          <div class="preview-surface" id="previewSurface">
            <div class="subtitle-list" id="previewList"></div>
          </div>
        </div>
      </section>

      <section class="settings-section log-section">
        <div class="section-title-row">
          <h2>日誌</h2>
          <button class="secondary-button" id="clearLog" type="button">清除日誌</button>
        </div>
        <span class="last-error" id="lastError"></span>
        <pre class="log-text" id="logText"></pre>
      </section>
    </main>
  `;

  const shell = query<HTMLElement>(".settings-shell");
  const previewSurface = query<HTMLElement>("#previewSurface");
  const previewList = query<HTMLDivElement>("#previewList");
  const toggleStreamButton = query<HTMLButtonElement>("#toggleStream");
  const exitButton = query<HTMLButtonElement>("#exitButton");
  const clearLogButton = query<HTMLButtonElement>("#clearLog");
  const statusPill = query<HTMLDivElement>("#statusPill");
  const statusText = query<HTMLSpanElement>("#statusText");
  const serverUrlInput = query<HTMLInputElement>("#serverUrl");
  const originalColorInput = query<HTMLInputElement>("#originalColor");
  const translatedColorInput = query<HTMLInputElement>("#translatedColor");
  const backgroundColorInput = query<HTMLInputElement>("#backgroundColor");
  const opacityInput = query<HTMLInputElement>("#opacity");
  const fontSizeInput = query<HTMLInputElement>("#fontSize");
  const fontFamilyInput = query<HTMLInputElement>("#fontFamily");
  const fontFamilyOptions = query<HTMLDataListElement>("#fontFamilyOptions");
  const subtitleCountInput = query<HTMLInputElement>("#subtitleCount");
  const opacityValue = query<HTMLSpanElement>("#opacityValue");
  const fontSizeValue = query<HTMLSpanElement>("#fontSizeValue");
  const subtitleCountValue = query<HTMLSpanElement>("#subtitleCountValue");
  const logText = query<HTMLPreElement>("#logText");
  const lastError = query<HTMLSpanElement>("#lastError");

  const renderForm = () => {
    serverUrlInput.value = settings.serverUrl;
    originalColorInput.value = normalizeHex(settings.originalTextColor, "#81D4FA");
    translatedColorInput.value = normalizeHex(settings.translatedTextColor, "#FFD54F");
    backgroundColorInput.value = normalizeHex(settings.backgroundColor, "#000000");
    opacityInput.value = String(settings.backgroundOpacity);
    fontSizeInput.value = String(settings.fontSize);
    fontFamilyInput.value = normalizeFontFamily(settings.fontFamily);
    renderFontOptions(fontFamilyOptions, settings.fontFamily);
    subtitleCountInput.value = String(settings.maxSubtitleCount);

    opacityValue.textContent = `${settings.backgroundOpacity}%`;
    fontSizeValue.textContent = `${Math.round(settings.fontSize)}px`;
    subtitleCountValue.textContent = `${settings.maxSubtitleCount} 組`;
    applyAppearance(shell);
    applyAppearance(previewSurface);
  };

  const renderConnection = () => {
    const labels: Record<ConnectionState, string> = {
      disconnected: "未連線",
      connecting: "連線中",
      connected: "已連線",
      reconnecting: "重連中",
      error: "錯誤",
    };

    statusText.textContent = labels[runtime.connectionState] ?? runtime.connectionState;
    statusPill.dataset.state = runtime.connectionState;
    toggleStreamButton.textContent = runtime.running ? "停止" : "啟動";
    toggleStreamButton.classList.toggle("stop", runtime.running);
  };

  const renderPreview = () => {
    previewList.innerHTML = renderSubtitleEntries(
      sampleSubtitles.slice(0, settings.maxSubtitleCount),
    );
  };

  const renderLogs = () => {
    logText.textContent = runtime.logs.join("\n");
    logText.scrollTop = logText.scrollHeight;
  };

  const renderLastError = () => {
    lastError.textContent = runtime.lastError ?? "";
  };

  const persistSettingsFromForm = async () => {
    settings = {
      ...settings,
      serverUrl: serverUrlInput.value.trim(),
      originalTextColor: originalColorInput.value,
      translatedTextColor: translatedColorInput.value,
      backgroundColor: backgroundColorInput.value,
      backgroundOpacity: Number(opacityInput.value),
      fontSize: Number(fontSizeInput.value),
      fontFamily: normalizeFontFamily(fontFamilyInput.value),
      maxSubtitleCount: Number(subtitleCountInput.value),
    };

    renderForm();
    renderPreview();
    await saveSettings();
  };

  toggleStreamButton.addEventListener("click", async () => {
    await persistSettingsFromForm();
    if (!isTauriRuntime) {
      runtime.running = !runtime.running;
      runtime.connectionState = runtime.running ? "connected" : "disconnected";
      runtime.logs.push(
        runtime.running ? "[preview] 模擬啟動字幕串流" : "[preview] 模擬停止字幕串流",
      );
      renderConnection();
      renderLogs();
      return;
    }

    if (runtime.running) {
      await invoke("stop_stream");
    } else {
      await invoke("start_stream");
    }
  });

  exitButton.addEventListener("click", async () => {
    if (isTauriRuntime) {
      await invoke("exit_app");
    }
  });

  clearLogButton.addEventListener("click", () => {
    runtime.logs = [];
    renderLogs();
  });

  serverUrlInput.addEventListener("change", () => void persistSettingsFromForm());
  originalColorInput.addEventListener("input", () => void persistSettingsFromForm());
  translatedColorInput.addEventListener("input", () => void persistSettingsFromForm());
  backgroundColorInput.addEventListener("input", () => void persistSettingsFromForm());
  opacityInput.addEventListener("input", () => void persistSettingsFromForm());
  fontSizeInput.addEventListener("input", () => void persistSettingsFromForm());
  fontFamilyInput.addEventListener("change", () => void persistSettingsFromForm());
  subtitleCountInput.addEventListener("input", () => void persistSettingsFromForm());

  renderUi = () => {
    renderForm();
    renderConnection();
    renderPreview();
    renderLogs();
    renderLastError();
  };
}

async function setupOverlayWindow() {
  document.documentElement.classList.add("overlay-document");
  document.body.classList.add("overlay-document");
  appRoot.innerHTML = `
    <main class="overlay-shell">
      <section class="overlay-subtitle-surface" id="subtitleSurface" data-tauri-drag-region>
        <div class="waiting" id="waitingText" data-tauri-drag-region>等待字幕串流...</div>
        <div class="subtitle-list" id="subtitleList"></div>
      </section>

      <button class="resize-handle resize-right" data-direction="East" type="button" title="調整寬度"></button>
      <button class="resize-handle resize-bottom" data-direction="South" type="button" title="調整高度"></button>
      <button class="resize-handle resize-corner" data-direction="SouthEast" type="button" title="調整大小"></button>
    </main>
  `;

  const shell = query<HTMLElement>(".overlay-shell");
  const subtitleSurface = query<HTMLDivElement>("#subtitleSurface");
  const subtitleList = query<HTMLDivElement>("#subtitleList");
  const waitingText = query<HTMLDivElement>("#waitingText");

  await prepareTransparentOverlay();

  let resizeFrameTimer = 0;
  const showResizeFrame = (active: boolean) => {
    window.clearTimeout(resizeFrameTimer);
    shell.classList.toggle("is-resizing", active);

    if (!active) {
      resizeFrameTimer = window.setTimeout(() => {
        shell.classList.remove("is-resizing");
      }, 420);
    }
  };

  document.querySelectorAll<HTMLButtonElement>(".resize-handle").forEach((handle) => {
    handle.addEventListener("pointerdown", async (event) => {
      event.preventDefault();
      showResizeFrame(true);

      const hideResizeFrame = () => showResizeFrame(false);
      window.addEventListener("pointerup", hideResizeFrame, { once: true });
      window.addEventListener("blur", hideResizeFrame, { once: true });
      window.setTimeout(hideResizeFrame, 1600);

      if (!appWindow) {
        return;
      }

      const direction = (handle.dataset.direction ?? "SouthEast") as ResizeDirection;
      try {
        await appWindow.startResizeDragging(direction);
      } catch (error) {
        hideResizeFrame();
        console.error(error);
      }
    });
  });

  window.addEventListener("beforeunload", () => {
    void persistOverlayBounds();
  });

  let boundsSaveTimer = 0;
  const scheduleBoundsSave = () => {
    window.clearTimeout(boundsSaveTimer);
    boundsSaveTimer = window.setTimeout(() => void persistOverlayBounds(), 250);
  };
  void appWindow?.onMoved(scheduleBoundsSave);
  void appWindow?.onResized(scheduleBoundsSave);

  await applySavedBounds();

  renderUi = () => {
    applyAppearance(shell);
    renderOverlaySubtitles(subtitleList, waitingText);
    subtitleSurface.scrollTop = subtitleSurface.scrollHeight;
  };
}

async function prepareTransparentOverlay() {
  document.documentElement.style.background = "transparent";
  document.body.style.background = "transparent";
  appRoot.style.background = "transparent";

  if (!appWindow || !appWebview || viewMode !== "overlay") {
    return;
  }

  await Promise.allSettled([
    appWebview.setBackgroundColor([0, 0, 0, 0]),
  ]);
}

function renderOverlaySubtitles(
  subtitleList: HTMLDivElement,
  waitingText: HTMLDivElement,
) {
  const previewEntries = isTauriRuntime ? [] : sampleSubtitles.slice(0, settings.maxSubtitleCount);
  const entries = runtime.subtitles.length ? runtime.subtitles : previewEntries;

  waitingText.hidden = entries.length > 0;
  subtitleList.innerHTML = renderSubtitleEntries(entries);
}

function renderSubtitleEntries(entries: SubtitleEntry[]) {
  return entries
    .map((entry, index) => {
      const divider = index < entries.length - 1 ? `<div class="subtitle-divider"></div>` : "";
      return `
        <article class="subtitle-entry">
          ${entry.original ? `<p class="subtitle-original">${escapeHtml(entry.original)}</p>` : ""}
          ${entry.translated ? `<p class="subtitle-translated">${escapeHtml(entry.translated)}</p>` : ""}
        </article>
        ${divider}
      `;
    })
    .join("");
}

async function saveSettings() {
  if (isTauriRuntime) {
    await invoke("save_settings", { settings });
  }
}

async function applySavedBounds() {
  if (!appWindow || viewMode !== "overlay") {
    return;
  }

  const bounds = settings.overlayBounds;
  await appWindow.setSize(new PhysicalSize(bounds.width, bounds.height));
  if (bounds.x !== null && bounds.y !== null) {
    await appWindow.setPosition(new PhysicalPosition(bounds.x, bounds.y));
  }
}

async function persistOverlayBounds() {
  if (!appWindow || viewMode !== "overlay") {
    return;
  }

  const position = await appWindow.outerPosition();
  const size = await appWindow.outerSize();
  const bounds: OverlayBounds = {
    x: position.x,
    y: position.y,
    width: size.width,
    height: size.height,
  };
  settings.overlayBounds = bounds;
  await invoke("save_overlay_bounds", { bounds });
}

function applyAppearance(element: HTMLElement) {
  const background = hexToRgb(settings.backgroundColor);
  element.style.setProperty("--original-color", settings.originalTextColor);
  element.style.setProperty("--translated-color", settings.translatedTextColor);
  element.style.setProperty("--subtitle-font-size", `${settings.fontSize}px`);
  element.style.setProperty("--subtitle-font-family", normalizeFontFamily(settings.fontFamily));
  element.style.setProperty(
    "--overlay-background",
    `rgba(${background.r}, ${background.g}, ${background.b}, ${settings.backgroundOpacity / 100})`,
  );
}

function normalizeFontFamily(value: string | undefined) {
  const normalized = (value ?? "").trim().replace(/\s+/g, " ");
  return normalized || "Sans";
}

async function loadSystemFontFamilies() {
  try {
    const families = await invoke<string[]>("list_system_fonts");
    return normalizeFontFamilies([...fallbackFontFamilies, ...families]);
  } catch (error) {
    console.error(error);
    return [...fallbackFontFamilies];
  }
}

function normalizeFontFamilies(families: string[]) {
  const normalized = families
    .map((family) => normalizeFontFamily(family))
    .filter((family) => family.length > 0);
  return [...new Set(normalized)].sort((a, b) => a.localeCompare(b));
}

function renderFontOptions(datalist: HTMLDataListElement, selectedFamily: string) {
  const families = normalizeFontFamilies([
    ...availableFontFamilies,
    normalizeFontFamily(selectedFamily),
  ]);

  datalist.innerHTML = families
    .map((family) => `<option value="${escapeHtml(family)}"></option>`)
    .join("");
}

function normalizeHex(value: string, fallback: string) {
  return /^#[0-9a-f]{6}$/i.test(value) ? value : fallback;
}

function hexToRgb(value: string) {
  const hex = normalizeHex(value, "#000000").slice(1);
  return {
    r: Number.parseInt(hex.slice(0, 2), 16),
    g: Number.parseInt(hex.slice(2, 4), 16),
    b: Number.parseInt(hex.slice(4, 6), 16),
  };
}

function escapeHtml(value: string) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function query<T extends Element = Element>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) {
    throw new Error(`Missing element: ${selector}`);
  }
  return element;
}
