use crate::{
    backends::{BackendExit, BackendRunContext, FocusBackend},
    config::WindowInfo,
    errors::DynError,
};
use std::{
    cell::RefCell, future::Future, path::Path, pin::Pin, sync::mpsc, thread, time::Duration,
};
use tokio::sync::mpsc::{Receiver, Sender, channel};
use windows::Win32::{
    Foundation::{CloseHandle, HWND, LPARAM, WPARAM},
    System::Threading::{
        GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    },
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        WindowsAndMessaging::{
            DispatchMessageW, EVENT_OBJECT_NAMECHANGE, EVENT_SYSTEM_FOREGROUND,
            GetForegroundWindow, GetMessageW, GetWindowTextLengthW, GetWindowTextW,
            GetWindowThreadProcessId, MSG, OBJID_WINDOW, PM_NOREMOVE, PeekMessageW,
            PostThreadMessageW, TranslateMessage, WINEVENT_OUTOFCONTEXT, WM_QUIT,
        },
    },
};

// OUTOFCONTEXT callbacks execute on the registering thread. Per-thread storage
// lets independent watchers start/stop without overwriting each other's sender.
thread_local! { static EVENTS: RefCell<Option<Sender<()>>> = const { RefCell::new(None) }; }

fn normalize_process_name(full_path: &str) -> Option<String> {
    Some(
        Path::new(full_path)
            .file_stem()?
            .to_string_lossy()
            .to_lowercase()
            .replace(' ', "_"),
    )
}

pub(crate) fn current_window_info_static() -> WindowInfo {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return WindowInfo::default();
        }
        let mut title = vec![0u16; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
        let len = GetWindowTextW(hwnd, &mut title).max(0) as usize;
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        WindowInfo {
            class: get_process_name(pid).unwrap_or_default(),
            title: String::from_utf16_lossy(&title[..len]),
            is_native_terminal: false,
        }
    }
}

fn get_process_name(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        // Maximum Windows extended path length, rather than truncating at 512 chars.
        let mut buffer = vec![0u16; 32768];
        let mut len = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        result.ok()?;
        normalize_process_name(&String::from_utf16_lossy(&buffer[..len as usize]))
    }
}

unsafe extern "system" fn foreground_event_callback(
    _: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    // Query the current foreground window when consumed; old queued events cannot
    // restore a stale app. Ignore title changes belonging to background/child objects.
    if event == EVENT_OBJECT_NAMECHANGE
        && (object != OBJID_WINDOW.0 || hwnd != unsafe { GetForegroundWindow() })
    {
        return;
    }
    EVENTS.with(|slot| {
        if let Some(sender) = slot.borrow().as_ref() {
            let _ = sender.try_send(());
        }
    });
}

struct Hooks(Vec<HWINEVENTHOOK>);
impl Drop for Hooks {
    fn drop(&mut self) {
        for hook in self.0.drain(..) {
            unsafe {
                let _ = UnhookWinEvent(hook);
            }
        }
    }
}
struct ForegroundWatcher {
    thread_id: u32,
    thread: Option<thread::JoinHandle<()>>,
}
impl ForegroundWatcher {
    fn start() -> Result<(Receiver<()>, Self), DynError> {
        let (events, receiver) = channel(1);
        let (ready, initialized) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("windows-focus".into())
            .spawn(move || unsafe {
                // PostThreadMessage requires the destination queue to exist BEFORE readiness.
                let mut msg = MSG::default();
                let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
                EVENTS.with(|slot| *slot.borrow_mut() = Some(events));
                let mut hooks = Hooks(Vec::new());
                for event in [EVENT_SYSTEM_FOREGROUND, EVENT_OBJECT_NAMECHANGE] {
                    let hook = SetWinEventHook(
                        event,
                        event,
                        None,
                        Some(foreground_event_callback),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT,
                    );
                    if hook.0.is_null() {
                        let _ = ready.send(Err("SetWinEventHook failed".to_string()));
                        EVENTS.with(|slot| *slot.borrow_mut() = None);
                        return;
                    }
                    hooks.0.push(hook);
                }
                if ready.send(Ok(GetCurrentThreadId())).is_ok() {
                    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                EVENTS.with(|slot| *slot.borrow_mut() = None);
            })?;
        match initialized.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(thread_id)) => Ok((
                receiver,
                Self {
                    thread_id,
                    thread: Some(thread),
                },
            )),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error.into())
            }
            Err(error) => Err(format!("Windows hook initialization failed: {error}").into()),
        }
    }
}
impl Drop for ForegroundWatcher {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
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
            let (events, _watcher) = ForegroundWatcher::start()?;
            println!("[Windows] Listening for foreground and window-title events");
            super::native::run(ctx, events, current_window_info_static).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_process_name;
    #[test]
    fn watcher_can_be_stopped_immediately_and_restarted() {
        for _ in 0..5 {
            let (_events, watcher) = super::ForegroundWatcher::start().unwrap();
            drop(watcher);
        }
    }
    #[test]
    fn normalize_process_name_handles_case_spaces_and_extension() {
        assert_eq!(
            normalize_process_name(r"C:\Program Files\My App\My App.EXE"),
            Some("my_app".into())
        );
        assert_eq!(
            normalize_process_name(r"C:\Windows\notepad.exe"),
            Some("notepad".into())
        );
    }
}
