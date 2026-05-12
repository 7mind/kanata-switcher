// Linux console backend placeholder.
// `run_linux_console_backend_task` remains in `supervisor/mod.rs` until PR-12
// absorbs it into the `FocusBackend` trait impl, because it depends on
// `BackendContext` (a supervisor type) and moving it here would create a
// backends → supervisor → backends cycle.
