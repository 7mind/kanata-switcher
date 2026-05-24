use super::*;

#[test]
fn test_apply_logind_display_change_reattaches_for_new_display_session_path() {
    let current =
        OwnedObjectPath::try_from("/org/freedesktop/login1/session/_1").expect("valid object path");
    let next =
        OwnedObjectPath::try_from("/org/freedesktop/login1/session/_2").expect("valid object path");
    assert_eq!(
        apply_logind_display_change(
            &current,
            true,
            false,
            "",
            LogindDisplayPathChange::Path(next.clone())
        ),
        LogindDisplayChangeAction::Reattach(next)
    );
}

#[test]
fn test_apply_logind_display_change_ignores_same_display_session_path() {
    let current =
        OwnedObjectPath::try_from("/org/freedesktop/login1/session/_1").expect("valid object path");
    assert_eq!(
        apply_logind_display_change(
            &current,
            true,
            false,
            "",
            LogindDisplayPathChange::Path(current.clone())
        ),
        LogindDisplayChangeAction::Ignore
    );
}

#[test]
fn test_apply_logind_display_change_emits_no_session_when_display_disappears() {
    let current =
        OwnedObjectPath::try_from("/org/freedesktop/login1/session/_1").expect("valid object path");
    assert_eq!(
        apply_logind_display_change(
            &current,
            true,
            true,
            "wayland",
            LogindDisplayPathChange::Empty
        ),
        LogindDisplayChangeAction::EmitNoSessionAndDetach(LifecycleSnapshot {
            active: false,
            session_type: String::new(),
            session_kind: SessionKind::NoSession,
        })
    );
}

#[test]
fn test_apply_logind_display_change_detaches_stale_session_when_already_no_session() {
    let current =
        OwnedObjectPath::try_from("/org/freedesktop/login1/session/_1").expect("valid object path");
    assert_eq!(
        apply_logind_display_change(&current, true, false, "", LogindDisplayPathChange::Empty),
        LogindDisplayChangeAction::DetachSessionMonitor
    );
}

#[test]
fn test_apply_logind_display_change_reattaches_same_path_when_session_monitor_detached() {
    let current =
        OwnedObjectPath::try_from("/org/freedesktop/login1/session/_1").expect("valid object path");
    assert_eq!(
        apply_logind_display_change(
            &current,
            false,
            false,
            "",
            LogindDisplayPathChange::Path(current.clone())
        ),
        LogindDisplayChangeAction::Reattach(current)
    );
}
