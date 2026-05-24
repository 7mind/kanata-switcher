use super::*;
use proptest::prelude::*;

#[test]
fn test_derive_default_dbus_suffix_default_host_default_port() {
    assert_eq!(derive_default_dbus_suffix("127.0.0.1", 10000), "p10000");
}

#[test]
fn test_derive_default_dbus_suffix_default_host_non_default_port() {
    assert_eq!(derive_default_dbus_suffix("127.0.0.1", 22334), "p22334");
}

#[test]
fn test_derive_default_dbus_suffix_non_default_host() {
    assert_eq!(
        derive_default_dbus_suffix("192.168.1.2", 22334),
        "h192_168_1_2_p22334"
    );
}

#[test]
fn test_derive_default_dbus_suffix_host_with_dashes_and_uppercase() {
    assert_eq!(
        derive_default_dbus_suffix("MyKB-test", 10000),
        "hMyKB-test_p10000"
    );
}

#[test]
fn test_derive_default_dbus_suffix_ipv6_style_host() {
    let result = derive_default_dbus_suffix("::1", 10000);
    assert_eq!(result, "h__1_p10000");
    assert!(
        !result.chars().next().unwrap().is_ascii_digit(),
        "result must not start with a digit"
    );
}

#[test]
fn test_sanitize_dbus_suffix_alphanumeric_unchanged() {
    assert_eq!(sanitize_dbus_suffix("kinesis").unwrap(), "kinesis");
    assert_eq!(sanitize_dbus_suffix("AbC123").unwrap(), "AbC123");
}

#[test]
fn test_sanitize_dbus_suffix_replaces_special_chars() {
    assert_eq!(
        sanitize_dbus_suffix("my.kb:slot 1").unwrap(),
        "my_kb_slot_1"
    );
}

#[test]
fn test_sanitize_dbus_suffix_keeps_dashes() {
    assert_eq!(sanitize_dbus_suffix("foo-bar").unwrap(), "foo-bar");
}

#[test]
fn test_sanitize_dbus_suffix_leading_digit_gets_underscore() {
    assert_eq!(sanitize_dbus_suffix("1kb").unwrap(), "_1kb");
}

#[test]
fn test_sanitize_dbus_suffix_empty_rejected() {
    assert!(matches!(
        sanitize_dbus_suffix(""),
        Err(DbusSuffixError::Empty)
    ));
}

#[test]
fn test_sanitize_dbus_suffix_too_long_rejected() {
    let raw = "a".repeat(65);
    assert!(matches!(
        sanitize_dbus_suffix(&raw),
        Err(DbusSuffixError::TooLong { length: 65, limit: 64 })
    ));
}

#[test]
fn test_sanitize_dbus_suffix_at_max_length_accepted() {
    let raw = "a".repeat(64);
    let sanitized = sanitize_dbus_suffix(&raw).expect("64-char input must succeed");
    assert_eq!(sanitized.chars().count(), 64);
}

#[test]
fn test_sanitize_dbus_suffix_max_length_digit_start_rejected() {
    // 64-char input starting with a digit would expand to 65 chars after the
    // leading-underscore prepend, exceeding the cap. Must be rejected.
    let raw = "1".repeat(64);
    let result = sanitize_dbus_suffix(&raw);
    assert!(
        matches!(
            result,
            Err(DbusSuffixError::TooLong {
                length: 65,
                limit: 64,
            })
        ),
        "64-char digit-start input must overflow the cap, got {:?}",
        result
    );
}

proptest! {
    /// Property: any non-empty ASCII input within the length cap must sanitize
    /// successfully *unless* it triggers a digit-prepend overflow (input ==
    /// 64 chars with a leading digit). Successful outputs satisfy the
    /// `[A-Za-z0-9_-]` rules, do not start with a digit, fit within the cap,
    /// and round-trip through `effective_dbus_name` → `is_daemon_bus_name`.
    #[test]
    fn proptest_sanitize_dbus_suffix_invariants(input in ".{1,64}") {
        match sanitize_dbus_suffix(&input) {
            Ok(sanitized) => {
                prop_assert!(!sanitized.is_empty(), "sanitized output must be non-empty");
                prop_assert!(
                    sanitized.chars().count() <= MAX_DBUS_SUFFIX_LEN,
                    "sanitized length {} exceeds cap {}",
                    sanitized.chars().count(),
                    MAX_DBUS_SUFFIX_LEN
                );
                let invalid_char = sanitized
                    .chars()
                    .find(|c| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '-'));
                prop_assert!(
                    invalid_char.is_none(),
                    "invalid char {:?} in sanitized output {:?}",
                    invalid_char,
                    sanitized
                );
                let first = sanitized.chars().next().expect("non-empty");
                prop_assert!(
                    !first.is_ascii_digit(),
                    "sanitized output must not start with a digit, got {:?}",
                    sanitized
                );
                prop_assert!(
                    is_daemon_bus_name(&effective_dbus_name(&sanitized)),
                    "effective_dbus_name({:?}) must be recognized as a daemon name",
                    sanitized
                );
            }
            Err(DbusSuffixError::TooLong { length, limit }) => {
                // The only failure path reachable from a ≤64-char input is
                // the digit-prepend overflow: input chars().count() == 64 and
                // first char is a digit (which survives sanitization as a digit
                // and therefore triggers the underscore prepend).
                prop_assert_eq!(limit, MAX_DBUS_SUFFIX_LEN);
                prop_assert_eq!(length, MAX_DBUS_SUFFIX_LEN + 1);
                prop_assert_eq!(input.chars().count(), MAX_DBUS_SUFFIX_LEN);
                let first = input.chars().next().expect("non-empty input");
                prop_assert!(
                    first.is_ascii_digit(),
                    "TooLong from a 64-char input is only valid for digit-start; got {:?}",
                    input
                );
            }
            Err(other) => prop_assert!(
                false,
                "unexpected sanitize error {:?} for input {:?}",
                other,
                input
            ),
        }
    }
}

#[test]
fn test_resolve_dbus_suffix_explicit_value_wins() {
    let result = resolve_dbus_suffix(Some("kinesis"), "127.0.0.1", 10000).unwrap();
    assert_eq!(result, "kinesis");
}

#[test]
fn test_resolve_dbus_suffix_explicit_sanitization_applied() {
    let result = resolve_dbus_suffix(Some("kb.1"), "127.0.0.1", 10000).unwrap();
    assert_eq!(result, "kb_1");
}

#[test]
fn test_resolve_dbus_suffix_explicit_empty_errors() {
    assert!(resolve_dbus_suffix(Some(""), "127.0.0.1", 10000).is_err());
}

#[test]
fn test_resolve_dbus_suffix_defaults_to_derived_suffix() {
    assert_eq!(
        resolve_dbus_suffix(None, "127.0.0.1", 10000).unwrap(),
        "p10000"
    );
    assert_eq!(
        resolve_dbus_suffix(None, "127.0.0.1", 22334).unwrap(),
        "p22334"
    );
}

#[test]
fn test_effective_dbus_name_compose() {
    assert_eq!(
        effective_dbus_name("p10000"),
        "com.github.kanata.Switcher.instances.p10000"
    );
    assert_eq!(
        effective_dbus_name("kinesis"),
        "com.github.kanata.Switcher.instances.kinesis"
    );
}

#[test]
fn test_effective_dbus_name_round_trips_through_is_daemon_bus_name() {
    // Structural invariant — if the prefix used by `effective_dbus_name` and
    // `is_daemon_bus_name` ever drifts apart, this test catches it.
    for suffix in ["p10000", "kinesis", "h192_168_1_2_p22334", "_1kb", "a-b-c"] {
        let name = effective_dbus_name(suffix);
        assert!(
            is_daemon_bus_name(&name),
            "{} produced by effective_dbus_name({:?}) must be recognized",
            name,
            suffix
        );
    }
}

#[test]
fn test_is_daemon_bus_name_accepts_instances_subtree() {
    assert!(is_daemon_bus_name(
        "com.github.kanata.Switcher.instances.p10000"
    ));
    assert!(is_daemon_bus_name(
        "com.github.kanata.Switcher.instances.kinesis"
    ));
}

#[test]
fn test_is_daemon_bus_name_rejects_extensions_subtree() {
    assert!(!is_daemon_bus_name(
        "com.github.kanata.Switcher.extensions.GNOME"
    ));
}

#[test]
fn test_is_daemon_bus_name_rejects_root_and_bare_prefix() {
    assert!(!is_daemon_bus_name("com.github.kanata.Switcher"));
    assert!(!is_daemon_bus_name("com.github.kanata.Switcher.instances"));
}

#[test]
fn test_is_daemon_bus_name_rejects_unrelated_names() {
    assert!(!is_daemon_bus_name("org.gnome.Shell"));
    assert!(!is_daemon_bus_name("org.kde.KWin"));
    assert!(!is_daemon_bus_name(""));
}

#[test]
fn test_autostart_passthrough_args_includes_dbus_suffix() {
    let matches = Args::command().get_matches_from([
        "kanata-switcher",
        "--install-autostart",
        "--dbus-suffix",
        "kinesis",
    ]);
    let args = Args::from_arg_matches(&matches).unwrap();
    let exec_args = autostart_passthrough_args(&matches, &args);
    assert_eq!(
        exec_args,
        vec!["--dbus-suffix".to_string(), "kinesis".to_string()]
    );
}

#[test]
fn test_args_dbus_suffix_rejects_empty() {
    let result = Args::try_parse_from(["kanata-switcher", "--dbus-suffix", ""]);
    assert!(
        result.is_err(),
        "expected --dbus-suffix '' to error, got {:?}",
        result.ok().map(|args| args.dbus_suffix)
    );
}

#[test]
fn test_args_dbus_suffix_sanitization_applied() {
    let args = Args::parse_from(["kanata-switcher", "--dbus-suffix", "kb.1"]);
    assert_eq!(args.dbus_suffix.as_deref(), Some("kb_1"));
}

#[test]
fn test_args_dbus_suffix_at_max_length_boundary() {
    let exactly_max = "a".repeat(MAX_DBUS_SUFFIX_LEN);
    let args =
        Args::try_parse_from(["kanata-switcher", "--dbus-suffix", &exactly_max]).expect(
            "64-char suffix must be accepted by clap value parser",
        );
    assert_eq!(args.dbus_suffix.as_deref(), Some(exactly_max.as_str()));

    let overlong = "a".repeat(MAX_DBUS_SUFFIX_LEN + 1);
    let result = Args::try_parse_from(["kanata-switcher", "--dbus-suffix", &overlong]);
    assert!(
        result.is_err(),
        "65-char suffix must be rejected by clap value parser"
    );
}
