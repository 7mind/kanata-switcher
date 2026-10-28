#define UNICODE
#define _UNICODE
#include <windows.h>
#include <iostream>
#include <string>
#include <thread>

constexpr UINT command_message = WM_APP + 1;
LRESULT CALLBACK window_proc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam) {
    if (message == command_message) {
        if (wparam == 1) {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
            std::cout << (GetForegroundWindow() == hwnd ? "FOCUSED" : "ERROR focus denied") << std::endl;
        } else if (wparam == 2) {
            SetWindowTextW(hwnd, L"vim - native fixture");
        } else if (wparam == 3) {
            SetWindowTextW(hwnd, L"Native focus fixture");
        } else if (wparam == 4) {
            const auto granted = AllowSetForegroundWindow(static_cast<DWORD>(lparam));
            std::cout << (granted ? "GRANTED" : "ERROR foreground grant denied") << std::endl;
        } else {
            DestroyWindow(hwnd);
        }
        return 0;
    }
    if (message == WM_DESTROY) { PostQuitMessage(0); return 0; }
    return DefWindowProcW(hwnd, message, wparam, lparam);
}
int main() {
    const auto instance = GetModuleHandleW(nullptr);
    WNDCLASSW wc{};
    wc.lpfnWndProc = window_proc;
    wc.hInstance = instance;
    wc.lpszClassName = L"KanataNativeFocusFixture";
    if (!RegisterClassW(&wc)) return 1;
    const auto hwnd = CreateWindowW(wc.lpszClassName, L"Native focus fixture",
        WS_OVERLAPPEDWINDOW, 80, 80, 400, 200,
        nullptr, nullptr, instance, nullptr);
    if (!hwnd) return 2;
    // The test process is allowed to transfer foreground rights to this helper.
    std::thread([hwnd] {
        std::string command;
        while (std::getline(std::cin, command)) {
            const auto action = command == "focus" ? 1 : command == "vim" ? 2 :
                                command == "title" ? 3 : command.rfind("grant ", 0) == 0 ? 4 : 0;
            const auto target = action == 4 ? static_cast<LPARAM>(std::stoul(command.substr(6))) : 0;
            PostMessageW(hwnd, command_message, action, target);
            if (!action) return;
        }
        PostMessageW(hwnd, command_message, 0, 0);
    }).detach();
    std::cout << "READY" << std::endl;
    MSG msg{};
    int result;
    while ((result = GetMessageW(&msg, nullptr, 0, 0)) > 0) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    return result < 0 ? 3 : 0;
}
