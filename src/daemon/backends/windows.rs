use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{
    GetCurrentThreadId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{
    HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent, WINEVENT_OUTOFCONTEXT,
    WINEVENT_SKIPOWNPROCESS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EVENT_SYSTEM_FOREGROUND, GetForegroundWindow, GetMessageW, GetWindowTextW,
    GetWindowThreadProcessId, MSG, PostThreadMessageW, TranslateMessage, WM_QUIT,
};

use crate::backends::{BackendExit, BackendRunContext, FocusBackend};
use crate::config::WindowInfo;
use crate::errors::DynError;
use crate::focus::FocusHandler;
use crate::focus_pipeline::{execute_focus_actions, handle_focus_event};
use crate::kanata::KanataClient;
use crate::broadcasters::{PauseBroadcaster, ShutdownHandle, StatusBroadcaster};

static FOREGROUND_EVENT_SENDER: OnceLock<Mutex<Option<UnboundedSender<WindowInfo>>>> =
    OnceLock::new();

fn foreground_event_sender() -> &'static Mutex<Option<UnboundedSender<WindowInfo>>> {
    FOREGROUND_EVENT_SENDER.get_or_init(|| Mutex::new(None))
}

fn normalize_process_name(full_path: &str) -> Option<String> {
    Some(
        Path::new(full_path)
            .file_stem()?
            .to_string_lossy()
            .to_lowercase()
            .replace(' ', "_"),
    )
}

unsafe fn current_window_info_for(hwnd: HWND) -> WindowInfo {
    if hwnd.0.is_null() {
        return WindowInfo::default();
    }

    let mut title_buf = [0u16; 512];
    let title_len = GetWindowTextW(hwnd, &mut title_buf) as usize;
    let title = String::from_utf16_lossy(&title_buf[..title_len]);

    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    let class = if pid == 0 {
        String::new()
    } else {
        get_process_name(pid).unwrap_or_default()
    };

    WindowInfo {
        class,
        title,
        is_native_terminal: false,
    }
}

pub(crate) fn current_window_info_static() -> WindowInfo {
    unsafe { current_window_info_for(GetForegroundWindow()) }
}

fn current_window_info() -> WindowInfo {
    unsafe { current_window_info_for(GetForegroundWindow()) }
}

fn get_process_name(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        if QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_err()
        {
            let _ = CloseHandle(handle);
            return None;
        }

        let full_path = String::from_utf16_lossy(&buf[..len as usize]);
        let normalized = normalize_process_name(&full_path);
        let _ = CloseHandle(handle);
        normalized
    }
}

unsafe extern "system" fn foreground_event_callback(
    _: HWINEVENTHOOK,
    _: u32,
    hwnd: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    let info = current_window_info_for(hwnd);
    if let Ok(guard) = foreground_event_sender().lock() {
        if let Some(sender) = guard.as_ref() {
            let _ = sender.send(info);
        }
    }
}

fn spawn_foreground_window_stream() -> (UnboundedReceiver<WindowInfo>, Option<u32>) {
    let (event_tx, event_rx) = unbounded_channel();
    let (thread_id_tx, thread_id_rx) = std::sync::mpsc::channel();

    thread::spawn(move || unsafe {
        {
            let mut slot = foreground_event_sender().lock().unwrap();
            *slot = Some(event_tx);
        }

        let thread_id = GetCurrentThreadId();
        let _ = thread_id_tx.send(thread_id);

        let hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(foreground_event_callback),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );

        let mut msg = MSG::default();
        loop {
            let result = GetMessageW(&mut msg, None, 0, 0);
            let value = result.0;
            if value == 0 || value == -1 {
                break;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        if hook.0 != 0 {
            let _ = UnhookWinEvent(hook);
        }
        if let Ok(mut slot) = foreground_event_sender().lock() {
            *slot = None;
        }
    });

    (event_rx, thread_id_rx.recv().ok())
}

async fn run_windows(
    kanata: KanataClient,
    handler: Arc<Mutex<FocusHandler>>,
    status_broadcaster: StatusBroadcaster,
    pause_broadcaster: PauseBroadcaster,
    shutdown_handle: ShutdownHandle,
) -> Result<(), DynError> {
    println!("[Windows] Starting focus watcher (WinEvent foreground hook)");

    let initial = current_window_info();
    println!(
        "[Windows] Initial foreground window: class=\"{}\" title=\"{}\"",
        initial.class, initial.title
    );
    let default_layer = kanata.default_layer_sync();
    if let Some(actions) = handle_focus_event(
        &handler,
        &status_broadcaster,
        &pause_broadcaster,
        &initial,
        &kanata,
        &default_layer,
    )
    .await
    {
        execute_focus_actions(&kanata, actions).await;
    }

    let (mut event_rx, thread_id) = spawn_foreground_window_stream();
    let mut shutdown_rx = shutdown_handle.subscribe();

    println!("[Windows] Listening for foreground-window events...");

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => {
                println!("[Windows] Shutting down focus watcher");
                if let Some(thread_id) = thread_id {
                    unsafe {
                        let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
                    }
                }
                return Ok(());
            }
            maybe_window = event_rx.recv() => {
                let Some(win) = maybe_window else {
                    return Ok(());
                };

                let default_layer = kanata.default_layer_sync();
                if let Some(actions) = handle_focus_event(
                    &handler,
                    &status_broadcaster,
                    &pause_broadcaster,
                    &win,
                    &kanata,
                    &default_layer,
                )
                .await
                {
                    execute_focus_actions(&kanata, actions).await;
                }
            }
        }
    }
}

pub(crate) struct WindowsBackend;

impl FocusBackend for WindowsBackend {
    fn run(
        self: Box<Self>,
        ctx: BackendRunContext,
    ) -> Pin<Box<dyn Future<Output = Result<BackendExit, DynError>> + Send + 'static>> {
        Box::pin(async move {
            run_windows(
                ctx.kanata,
                ctx.focus_handler,
                ctx.status_broadcaster,
                ctx.pause_broadcaster,
                ctx.shutdown_handle,
            )
            .await?;
            Ok(BackendExit::Exit)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_process_name;

    #[test]
    fn normalize_process_name_lowercases_and_strips_extension() {
        assert_eq!(
            normalize_process_name("C:\\Program Files\\Firefox\\Firefox.exe"),
            Some("firefox".to_string())
        );
        assert_eq!(
            normalize_process_name("C:\\Windows\\System32\\notepad.EXE"),
            Some("notepad".to_string())
        );
    }

    #[test]
    fn normalize_process_name_replaces_spaces() {
        assert_eq!(
            normalize_process_name("C:\\Program Files\\My App\\My App.exe"),
            Some("my_app".to_string())
        );
    }
}
