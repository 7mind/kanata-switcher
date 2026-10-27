pub(crate) mod settings;
pub(crate) mod state;
pub(crate) mod indicator;
pub(crate) mod control_local;
pub(crate) mod control_dbus;
pub(crate) mod control_ops;
pub(crate) mod guard;

pub(crate) use settings::*;
pub(crate) use indicator::*;
pub(crate) use control_local::*;
pub(crate) use control_dbus::*;
pub(crate) use guard::*;

use std::time::Duration;
use tokio::sync::watch;
use crate::environ::Environment;

#[derive(Clone)]
pub(crate) enum SniControl {
    Local(SniLocalControl),
    Dbus(SniDbusControl),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SniControlMode {
    Local,
    Dbus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SniRuntimeTransitionPlan {
    Keep,
    Stop,
    Start(SniControlMode),
    Restart(SniControlMode),
}

pub(crate) fn sni_control_mode_for_environment(env: Environment) -> Option<SniControlMode> {
    match env {
        Environment::Wayland | Environment::X11 => Some(SniControlMode::Local),
        Environment::Kde => Some(SniControlMode::Dbus),
        Environment::Gnome | Environment::LinuxConsoleWithLogind | Environment::Unknown => None,
    }
}

pub(crate) fn plan_sni_runtime_transition(
    active_mode: Option<SniControlMode>,
    active_env: Option<Environment>,
    desired_env: Environment,
) -> SniRuntimeTransitionPlan {
    let desired_mode = sni_control_mode_for_environment(desired_env);
    match (active_mode, desired_mode) {
        (None, None) => SniRuntimeTransitionPlan::Keep,
        (Some(_), None) => SniRuntimeTransitionPlan::Stop,
        (None, Some(mode)) => SniRuntimeTransitionPlan::Start(mode),
        (Some(current_mode), Some(next_mode)) => {
            if current_mode != next_mode || active_env != Some(desired_env) {
                SniRuntimeTransitionPlan::Restart(next_mode)
            } else {
                SniRuntimeTransitionPlan::Keep
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SniRuntimeWakeReason {
    EnvironmentChanged,
    Retry,
    ChannelClosed,
}

pub(crate) async fn wait_for_sni_runtime_wake_with_delay(
    env_receiver: &mut watch::Receiver<Environment>,
    should_retry_start: bool,
    retry_delay: Duration,
) -> SniRuntimeWakeReason {
    if !should_retry_start {
        return match env_receiver.changed().await {
            Ok(_) => SniRuntimeWakeReason::EnvironmentChanged,
            Err(_) => SniRuntimeWakeReason::ChannelClosed,
        };
    }

    tokio::select! {
        changed = env_receiver.changed() => {
            match changed {
                Ok(_) => SniRuntimeWakeReason::EnvironmentChanged,
                Err(_) => SniRuntimeWakeReason::ChannelClosed,
            }
        }
        _ = tokio::time::sleep(retry_delay) => SniRuntimeWakeReason::Retry,
    }
}
