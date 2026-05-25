//! Integration tests for desktop environment backends.
//!
//! These tests verify the integration between the daemon and various DE backends:
//! - DBus service (GNOME/KDE) — Linux only
//! - Wayland protocol (wlr-foreign-toplevel-management) — Linux only
//! - X11 PropertyNotify (requires Xvfb) — Linux only
//! - NSWorkspace focus pipeline — macOS only
//! - WinEvent focus pipeline — Windows only
//!
//! Tests requiring external dependencies (Xvfb, dbus-daemon) fail with helpful error
//! messages when dependencies are not available. Run via `nix run .#test` for guaranteed
//! full test coverage, or install dependencies manually.

use super::*;

#[cfg(target_os = "linux")]
use std::future::Future;
#[cfg(target_os = "linux")]
use std::io::{BufRead, BufReader, Write};
#[cfg(target_os = "linux")]
use std::net::TcpListener;
#[cfg(target_os = "linux")]
use std::sync::mpsc;
#[cfg(target_os = "linux")]
use std::thread;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

mod common;
pub(super) use common::*;

#[cfg(target_os = "linux")]
mod gnome;
#[cfg(target_os = "linux")]
mod kde;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
mod x11;
#[cfg(target_os = "linux")]
mod vk_validation;
#[cfg(target_os = "linux")]
mod dbus_control;
#[cfg(target_os = "linux")]
mod dbus_session_status;
#[cfg(target_os = "linux")]
mod dbus_session_restart;
#[cfg(target_os = "linux")]
mod dbus_session_pause;
#[cfg(target_os = "linux")]
mod dbus_session_persistent;
#[cfg(target_os = "linux")]
mod dbus_multiplex;
#[cfg(target_os = "linux")]
mod dconf;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
