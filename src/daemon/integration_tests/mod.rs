//! Integration tests for desktop environment backends.
//!
//! These tests verify the integration between the daemon and various DE backends:
//! - DBus service (GNOME/KDE)
//! - Wayland protocol (wlr-foreign-toplevel-management)
//! - X11 PropertyNotify (requires Xvfb)
//!
//! Tests requiring external dependencies (Xvfb, dbus-daemon) fail with helpful error
//! messages when dependencies are not available. Run via `nix run .#test` for guaranteed
//! full test coverage, or install dependencies manually.

use super::*;
use std::future::Future;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};


mod common;
pub(super) use common::*;

mod gnome;
mod kde;
mod wayland;
mod x11;
mod vk_validation;
mod dbus_control;
mod dbus_session_status;
mod dbus_session_restart;
mod dbus_session_pause;
mod dbus_session_persistent;
mod dbus_multiplex;
mod dconf;
