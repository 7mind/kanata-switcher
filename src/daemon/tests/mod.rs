use super::*;
use clap::Parser;
use proptest::prelude::*;
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};
use zbus::Message;
use zbus::zvariant::OwnedObjectPath;


mod common;
pub(crate) use common::*;

mod focus_flow;
mod focus_pipeline;
mod focus_property;
mod autostart;
mod dbus_naming;
mod control_commands;
mod kde_script_paths;
mod sni_presentation;
mod gnome_ext_state;
mod config_parsing;
mod lifecycle;
