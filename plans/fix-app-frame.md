Implement the frosted glass transparency fix for the Tauri window.



Context: The app currently has `decorations: true, transparent: false` which kills the frosted glass aesthetic. The fix requires three separate changes targeting three separate root causes.



\## What to do



\### 1. `src-tauri/Cargo.toml`

Add under `\[target.'cfg(target\_os = "windows")'.dependencies]`:

```toml

window-vibrancy = "0.5"

webview2-com = "<version>"  # run `cargo tree | grep webview2-com` first and match exactly

```



\### 2. `src-tauri/tauri.conf.json`

In the main window config, set:

```json

"decorations": false,

"transparent": true,

"shadow": false

```



\### 3. `src-tauri/src/lib.rs`

In the `.setup()` hook, after getting the main window handle, add this block for Windows:



```rust

\#\[cfg(target\_os = "windows")]

{

&#x20;   use window\_vibrancy::apply\_acrylic;



&#x20;   // Re-apply after WebView2 init to win the timing race

&#x20;   win.set\_decorations(false)?;



&#x20;   // Kill WebView2's opaque white default background at the controller level

&#x20;   win.with\_webview(|webview| {

&#x20;       unsafe {

&#x20;           use webview2\_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2\_COLOR;

&#x20;           let \_ = webview.controller().SetDefaultBackgroundColor(

&#x20;               COREWEBVIEW2\_COLOR { A: 0, R: 0, G: 0, B: 0 }

&#x20;           );

&#x20;       }

&#x20;   })?;



&#x20;   // Apply OS-level DWM acrylic so backdrop-filter composites against real desktop pixels

&#x20;   apply\_acrylic(\&win, Some((34, 34, 34, 153)))

&#x20;       .expect("Acrylic not supported — requires Windows 10 1803+");

}

```



\### 4. Verify `global.css`

Confirm `html, body, #root` still have `background: transparent`. Do not add anything, just verify and report.



\### 5. Sidebar positioning

After making the above changes, check `Sidebar.module.css` — if `margin-top` was changed from `50px` to `32px` for the native title bar, revert it back to `50px` (the custom title bar is being restored).



\## Notes

\- Run `cargo tree | grep webview2-com` before touching Cargo.toml to get the exact version

\- Do not touch any Rust backend code (strategy engine, DSL, indicators, broker, IPC)

\- If `webview2-com` import path fails to compile, report the error verbatim before attempting fixes

