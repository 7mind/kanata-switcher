use std::future::Future;
use std::pin::Pin;
use std::ptr::NonNull;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::Duration;

use block2::StackBlock;
use core_foundation_sys::runloop::{CFRunLoopRunInMode, kCFRunLoopDefaultMode};
use objc2::rc::{Retained, autoreleasepool};
use objc2_app_kit::{
    NSRunningApplication, NSWorkspace, NSWorkspaceApplicationKey,
    NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::NSNotification;
use tokio::sync::mpsc::{Receiver, channel};

use crate::backends::{BackendExit, BackendRunContext, FocusBackend};
use crate::config::WindowInfo;
use crate::errors::DynError;

pub(crate) fn current_window_info() -> WindowInfo {
    autoreleasepool(|_| unsafe {
        WindowInfo {
            class: NSWorkspace::sharedWorkspace()
                .frontmostApplication()
                .and_then(|app| app.bundleIdentifier())
                .map(|s| s.to_string())
                .unwrap_or_default(),
            // NSWorkspace exposes app identity, not window titles (no Accessibility grant).
            title: String::new(),
            is_native_terminal: false,
        }
    })
}

fn window_info_from_notification(notification: NonNull<NSNotification>) -> WindowInfo {
    unsafe {
        let class = notification
            .as_ref()
            .userInfo()
            .and_then(|info| info.objectForKey(NSWorkspaceApplicationKey))
            .and_then(|value| Retained::cast::<NSRunningApplication>(value).bundleIdentifier())
            .map(|id| id.to_string());
        match class {
            Some(class) => WindowInfo {
                class,
                title: String::new(),
                is_native_terminal: false,
            },
            None => current_window_info(),
        }
    }
}

struct AppObserver {
    latest: Arc<Mutex<Option<WindowInfo>>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl AppObserver {
    fn start() -> Result<(Receiver<()>, Self), DynError> {
        let (events, receiver) = channel(1);
        let (ready, initialized) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let latest = Arc::new(Mutex::new(None));
        let observed = latest.clone();
        let thread = thread::Builder::new()
            .name("macos-focus".into())
            .spawn(move || {
                autoreleasepool(|_| unsafe {
                    let center = NSWorkspace::sharedWorkspace().notificationCenter();
                    let observer = center.addObserverForName_object_queue_usingBlock(
                        Some(NSWorkspaceDidActivateApplicationNotification),
                        None,
                        None,
                        &StackBlock::new(move |notification: NonNull<NSNotification>| {
                            // NSWorkspace's cached frontmostApplication may still name
                            // the previous app during this notification. Its payload is
                            // authoritative. Store the newest snapshot before waking.
                            *observed.lock().unwrap() =
                                Some(window_info_from_notification(notification));
                            let _ = events.try_send(());
                        }),
                    );
                    if ready.send(()).is_ok() {
                        while !stopping.load(Ordering::Acquire) {
                            // Bounded turns avoid a stop-before-run race and a dangling CFRunLoopRef.
                            // Some headless run loops have no sources: prevent a busy spin there.
                            autoreleasepool(|_| {
                                CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.05, 0);
                            });
                            thread::park_timeout(Duration::from_millis(10));
                        }
                    }
                    center.removeObserver(&observer);
                });
            })?;
        let guard = Self {
            latest,
            stop,
            thread: Some(thread),
        };
        initialized
            .recv_timeout(Duration::from_secs(5))
            .map_err(|error| format!("macOS observer initialization failed: {error}"))?;
        Ok((receiver, guard))
    }

    fn current(&self) -> WindowInfo {
        let latest = self.latest.lock().unwrap().clone();
        latest.unwrap_or_else(current_window_info)
    }
}
impl Drop for AppObserver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

pub(crate) struct MacOsBackend;
impl FocusBackend for MacOsBackend {
    fn run(
        self: Box<Self>,
        ctx: BackendRunContext,
    ) -> Pin<Box<dyn Future<Output = Result<BackendExit, DynError>> + Send + 'static>> {
        Box::pin(async move {
            let (events, observer) = AppObserver::start()?;
            println!("[macOS] Listening for frontmost-app activation events");
            super::native::run(ctx, events, || observer.current()).await
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn observer_can_be_stopped_immediately_and_restarted() {
        for _ in 0..5 {
            let (_events, observer) = super::AppObserver::start().unwrap();
            drop(observer);
        }
    }
}
