#include <windows.h>
#include <wrl.h>
#include <WebView2.h>
#include <string>
#include <windowsx.h>

using Microsoft::WRL::Callback;
using Microsoft::WRL::ComPtr;

namespace {
constexpr wchar_t kClassName[] = L"ZenBetaHudWindow";
constexpr wchar_t kHudUrl[] = L"http://127.0.0.1:23456/beta";
constexpr int kWidth = 1100;
constexpr int kHeight = 720;
constexpr UINT_PTR kRetryTimer = 7;

HWND g_window = nullptr;
ComPtr<ICoreWebView2Controller> g_controller;
ComPtr<ICoreWebView2> g_webview;
bool g_fullscreen = false;
bool g_pinned = false;
RECT g_windowed_bounds{};

void SetFixedWindowStyle(HWND hwnd) {
    LONG_PTR style = GetWindowLongPtrW(hwnd, GWL_STYLE);
    style &= ~(WS_THICKFRAME | WS_MAXIMIZEBOX);
    SetWindowLongPtrW(hwnd, GWL_STYLE, style);
    SetWindowPos(hwnd, nullptr, 0, 0, kWidth, kHeight,
                 SWP_NOMOVE | SWP_NOZORDER | SWP_FRAMECHANGED);
}

void ResizeWebView() {
    if (!g_controller || !g_window) return;
    RECT bounds{};
    GetClientRect(g_window, &bounds);
    g_controller->put_Bounds(bounds);
}

void HandleWebMessage(const std::wstring& message) {
    if (message == L"close") PostMessageW(g_window, WM_CLOSE, 0, 0);
    else if (message == L"minimize") ShowWindow(g_window, SW_MINIMIZE);
    else if (message == L"maximize") {
        if (g_fullscreen) {
            SetWindowPos(g_window, nullptr, g_windowed_bounds.left, g_windowed_bounds.top,
                         g_windowed_bounds.right - g_windowed_bounds.left,
                         g_windowed_bounds.bottom - g_windowed_bounds.top,
                         SWP_NOZORDER | SWP_FRAMECHANGED);
        } else {
            GetWindowRect(g_window, &g_windowed_bounds);
            HMONITOR monitor = MonitorFromWindow(g_window, MONITOR_DEFAULTTONEAREST);
            MONITORINFO info{sizeof(MONITORINFO)};
            GetMonitorInfoW(monitor, &info);
            const RECT& area = info.rcMonitor;
            SetWindowPos(g_window, HWND_TOP, area.left, area.top, area.right - area.left,
                         area.bottom - area.top, SWP_FRAMECHANGED);
        }
        g_fullscreen = !g_fullscreen;
    }
    else if (message == L"pin") {
        g_pinned = !g_pinned;
        SetWindowPos(g_window, g_pinned ? HWND_TOPMOST : HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
    }
}

void CreateWebView() {
    CreateCoreWebView2EnvironmentWithOptions(
        nullptr, nullptr, nullptr,
        Callback<ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler>(
            [](HRESULT result, ICoreWebView2Environment* environment) -> HRESULT {
                if (FAILED(result) || !environment) return result;
                return environment->CreateCoreWebView2Controller(
                    g_window,
                    Callback<ICoreWebView2CreateCoreWebView2ControllerCompletedHandler>(
                        [](HRESULT controller_result, ICoreWebView2Controller* controller) -> HRESULT {
                            if (FAILED(controller_result) || !controller) return controller_result;
                            g_controller = controller;
                            g_controller->get_CoreWebView2(&g_webview);
                            ResizeWebView();

                            ComPtr<ICoreWebView2Settings> settings;
                            g_webview->get_Settings(&settings);
                            if (settings) {
                                settings->put_IsStatusBarEnabled(FALSE);
                                settings->put_AreDefaultContextMenusEnabled(TRUE);
                                settings->put_IsZoomControlEnabled(FALSE);
                            }

                            g_webview->add_WebMessageReceived(
                                Callback<ICoreWebView2WebMessageReceivedEventHandler>(
                                    [](ICoreWebView2*, ICoreWebView2WebMessageReceivedEventArgs* args) -> HRESULT {
                                        LPWSTR raw = nullptr;
                                        if (SUCCEEDED(args->TryGetWebMessageAsString(&raw)) && raw) {
                                            HandleWebMessage(raw);
                                            CoTaskMemFree(raw);
                                        }
                                        return S_OK;
                                    }).Get(), nullptr);

                            g_webview->add_NavigationCompleted(
                                Callback<ICoreWebView2NavigationCompletedEventHandler>(
                                    [](ICoreWebView2*, ICoreWebView2NavigationCompletedEventArgs* args) -> HRESULT {
                                        BOOL success = FALSE;
                                        args->get_IsSuccess(&success);
                                        if (!success && g_window) {
                                            SetTimer(g_window, kRetryTimer, 500, nullptr);
                                        }
                                        return S_OK;
                                    }).Get(), nullptr);
                            g_webview->Navigate(kHudUrl);
                            return S_OK;
                        }).Get());
            }).Get());
}

LRESULT CALLBACK WindowProc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam) {
    switch (message) {
    case WM_CREATE:
        g_window = hwnd;
        SetFixedWindowStyle(hwnd);
        CreateWebView();
        return 0;
    case WM_SIZE:
        ResizeWebView();
        return 0;
    case WM_TIMER:
        if (wparam == kRetryTimer && g_webview) {
            KillTimer(hwnd, kRetryTimer);
            g_webview->Navigate(kHudUrl);
        }
        return 0;
    case WM_GETMINMAXINFO: {
        auto* info = reinterpret_cast<MINMAXINFO*>(lparam);
        info->ptMinTrackSize.x = kWidth;
        info->ptMinTrackSize.y = kHeight;
        info->ptMaxTrackSize.x = kWidth;
        info->ptMaxTrackSize.y = kHeight;
        return 0;
    }
    case WM_CLOSE:
        if (g_webview) {
            g_webview->ExecuteScript(
                L"navigator.sendBeacon('/command', new Blob([JSON.stringify({command:'close_beta'})], {type:'application/json'}));",
                nullptr);
        }
        DestroyWindow(hwnd);
        return 0;
    case WM_DESTROY:
        KillTimer(hwnd, kRetryTimer);
        g_webview.Reset();
        g_controller.Reset();
        PostQuitMessage(0);
        return 0;
    default:
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
}
}

int WINAPI wWinMain(HINSTANCE instance, HINSTANCE, PWSTR, int show_command) {
    if (FAILED(CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED))) return 1;

    WNDCLASSEXW wc{sizeof(WNDCLASSEXW)};
    wc.hInstance = instance;
    wc.lpfnWndProc = WindowProc;
    wc.lpszClassName = kClassName;
    wc.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    wc.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
    RegisterClassExW(&wc);

    RECT rect{0, 0, kWidth, kHeight};
    AdjustWindowRect(&rect, WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU, FALSE);
    g_window = CreateWindowExW(
        0, kClassName, L"ZEN HUD", WS_POPUP,
        CW_USEDEFAULT, CW_USEDEFAULT, rect.right - rect.left, rect.bottom - rect.top,
        nullptr, nullptr, instance, nullptr);
    if (!g_window) {
        CoUninitialize();
        return 1;
    }

    ShowWindow(g_window, SW_SHOW);
    UpdateWindow(g_window);

    MSG message{};
    while (GetMessageW(&message, nullptr, 0, 0) > 0) {
        TranslateMessage(&message);
        DispatchMessageW(&message);
    }

    CoUninitialize();
    return static_cast<int>(message.wParam);
}
