# 浮窗字幕 SubtitleOverlay

即時翻譯字幕浮窗應用程式，透過 SSE (Server-Sent Events) 接收翻譯伺服器的即時字幕串流，並以浮窗覆蓋在其他應用程式上方顯示。

目前包含：
- Android 版：系統級浮窗 + 前景服務。
- Desktop 版：Tauri v2 設定/控制視窗 + 透明置頂字幕 overlay。

> 本專案需搭配 [stream-translator-gpt-floatwindow-ui](https://github.com/SakurajimaMai-1202/stream-translator-gpt-floatwindow-ui) 使用。

## 效果展示

<p align="center">
  <img src="screenshots/main_ui.png" width="300" alt="主界面"/>
  &nbsp;&nbsp;&nbsp;&nbsp;
  <img src="screenshots/overlay.png" width="500" alt="浮窗效果"/>
</p>

## 功能特色

### 核心功能
- 🔗 **SSE 即時字幕串流** — 自動偵測翻譯任務、解析 subtitle/status/error/ping 事件
- 📱 **Android 系統浮窗** — `TYPE_APPLICATION_OVERLAY`，覆蓋在其他 App 上方
- 🖥️ **桌面透明置頂 Overlay** — Windows / macOS / Linux，設定視窗獨立控制字幕浮窗
- 📝 **多行字幕** — 原文 + 翻譯同時顯示，可自訂保留組數（1-10 組）
- 🔄 **自動重連** — 斷線後以指數退避方式自動重連
- 🔔 **前景服務** — 常駐通知，不會被系統殺掉

### 自訂功能
- 🎨 原文 / 翻譯文字顏色（色盤 + Hex 色碼）
- 🖌️ 背景顏色與透明度（0-100%）
- 🔤 字體大小（10-32sp）
- 📋 內建偵錯日誌面板

### 浮窗操作
- ✋ **拖曳移動** — 觸控中央區域拖曳
- ↔️ **調整大小** — 拖曳邊緣（PiP 風格），四邊＋四角均可
- 💾 **尺寸記憶** — 調整後自動儲存

## 系統需求

### Android

- Android 8.0 (API 26) 以上
- 需授予「顯示在其他應用程式上層」權限

### Desktop

- Windows / macOS / Linux
- Node.js 20+
- Rust stable
- Windows 需 WebView2 Runtime
- Linux 需 Tauri/WebKitGTK 系統依賴
- Linux 版會在啟動早期自動設定 `WEBKIT_DISABLE_COMPOSITING_MODE=1`（若使用者尚未設定），以避開部分 WebKitGTK/compositor 組合下透明 overlay 顯示成白底的問題。

## 連線設定

應用程式透過以下流程與翻譯伺服器對接：

1. `GET /api/translation/active-task` → 取得當前翻譯任務 ID
2. `GET /api/translation/stream/{task_id}` → SSE 即時字幕串流

只需輸入伺服器完整位址即可連線，支援 `http` 和 `https` 協定：

```
http://192.168.1.100:8765
https://my-server.example.com:8765
```

## 使用方式

### Android

1. 從 [Releases](https://github.com/W-Nana/SubtitleOverlay/releases) 下載 APK
2. 安裝到 Android 裝置
3. 授予「顯示在其他應用程式上層」(懸浮窗) 權限
4. 輸入翻譯伺服器位址（如 `http://192.168.1.100:8765`）
5. 調整字幕外觀設定
6. 點擊「啟動浮窗字幕」
7. 切換到其他 App，字幕會以浮窗形式覆蓋顯示

### Desktop

1. 進入 `desktop/`
2. 安裝依賴：`npm install`
3. 開發模式：`npm run tauri:dev`
4. 在設定視窗輸入翻譯伺服器位址並調整外觀
5. 點擊「啟動」開始接收字幕

桌面版由兩個視窗組成：
- 設定視窗提供啟動/停止、連線狀態、伺服器位址、外觀、預覽、日誌清除和退出。
- overlay 視窗只顯示字幕，透明、無邊框、置頂，可拖曳並可從右側、底部、右下角調整大小。
- 關閉設定視窗或點擊退出會結束桌面應用；不使用系統托盤。
- overlay 位置、尺寸與樣式設定會自動保存到使用者配置目錄。

## 建置

### Android

```bash
# 編譯 Debug APK
./gradlew assembleDebug

# 編譯 Release APK（需設定簽名）
./gradlew assembleRelease
```

Android 環境需求：
- Java 17+
- Android SDK (compileSdk 34)

### Desktop

```bash
cd desktop
npm install

# 前端 typecheck + build
npm run build:web

# 建置 release 二進制，不產生安裝器
npm run build

# macOS 建置 .app bundle，不產生 .dmg
npm run build:mac-app
```

Release 產物：
- Windows：`desktop/src-tauri/target/release/subtitle-overlay-desktop.exe`
- Linux：`desktop/src-tauri/target/release/subtitle-overlay-desktop`
- macOS：`desktop/src-tauri/target/release/bundle/macos/SubtitleOverlay.app`（macOS GUI 應用仍需 app bundle 結構）

桌面版預設停用 installer bundle；Windows/Linux 的 `npm run build` 使用 `tauri build --no-bundle`。macOS 使用平台設定只啟用 `app` bundle，不產生 `.dmg` 安裝鏡像。

## 技術細節

### Android

| 項目 | 內容 |
|------|------|
| 語言 | Kotlin |
| 最低版本 | Android 8.0 (API 26) |
| 目標版本 | Android 14 (API 34) |
| 網路 | OkHttp 4.12.0 (手動解析 SSE) |
| UI | Material Design 3 |
| 架構 | LifecycleService + WindowManager |

### Desktop

| 項目 | 內容 |
|------|------|
| 框架 | Tauri v2 |
| 後端 | Rust |
| 前端 | Vanilla TypeScript + CSS |
| 網路 | reqwest + 手動解析 SSE |
| 視窗 | 設定/控制視窗 + 透明、無邊框、置頂 overlay |
| 發佈 | Windows/Linux no-bundle 二進制；macOS `.app` bundle |

## 授權

MIT License
