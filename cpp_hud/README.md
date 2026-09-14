# Native Beta HUD host

This is a native C++20 Win32 host for the Beta HUD. It embeds Microsoft Edge WebView2 and loads the existing Rust server UI from:

`http://127.0.0.1:23456/beta`

The Rust application must be running first. The window is fixed at `1100x720`; resize and maximize are disabled.

## Requirements

- Visual Studio 2022 with Desktop development with C++
- CMake 3.21+
- WebView2 Runtime
- Microsoft WebView2 SDK (the current checkout uses `webview2_sdk`)

Set `WEBVIEW2_SDK_DIR` to the SDK root, then run from a Developer PowerShell:

```powershell
cmake -S . -B build -A x64
cmake --build build --config Release
```

To use another SDK location, set `WEBVIEW2_SDK_DIR` before running CMake.

Run `build\Release\zen_beta_hud.exe` after starting `zen_rpc`.
