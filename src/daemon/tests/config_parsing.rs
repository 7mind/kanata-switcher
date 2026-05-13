use super::*;

// === Config Parsing Tests ===

#[test]
fn test_config_rejects_unknown_fields() {
    // "native_terminal" is not a valid field (should be "on_native_terminal")
    let json = r#"[{"native_terminal": "tty"}]"#;
    let result: Result<Vec<ConfigEntry>, _> = serde_json::from_str(json);
    assert!(
        result.is_err(),
        "Config should reject unknown field 'native_terminal'"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("unknown field 'native_terminal'"),
        "Error should mention the unknown field name, got: {}",
        err
    );
}

#[test]
fn test_config_rejects_typo_in_field_name() {
    // Common typos should be rejected
    let json = r#"[{"clas": "firefox", "layer": "browser"}]"#;
    let result: Result<Vec<ConfigEntry>, _> = serde_json::from_str(json);
    assert!(result.is_err(), "Config should reject typo 'clas'");
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("unknown field 'clas'"),
        "Error should mention the typo field name, got: {}",
        err
    );
}

#[test]
fn test_config_accepts_valid_rule() {
    let json = r#"[{"class": "firefox", "layer": "browser"}]"#;
    let result: Result<Vec<ConfigEntry>, _> = serde_json::from_str(json);
    assert!(result.is_ok(), "Config should accept valid rule");
}

#[test]
fn test_config_accepts_on_native_terminal() {
    let json = r#"[{"on_native_terminal": "tty"}]"#;
    let result: Result<Vec<ConfigEntry>, _> = serde_json::from_str(json);
    assert!(
        result.is_ok(),
        "Config should accept 'on_native_terminal' field"
    );
}

#[test]
fn test_config_accepts_default_entry() {
    let json = r#"[{"default": "base"}]"#;
    let result: Result<Vec<ConfigEntry>, _> = serde_json::from_str(json);
    assert!(result.is_ok(), "Config should accept default entry");
}

#[test]
fn test_config_parses_matcherless_rule_with_fallthrough() {
    // A rule with no class/title but with fallthrough: true is valid
    // (applies to all windows and continues matching)
    let json = r#"[{"layer": "base", "fallthrough": true}]"#;
    let result: Result<Vec<ConfigEntry>, _> = serde_json::from_str(json);
    assert!(
        result.is_ok(),
        "Config should parse matcher-less rule with fallthrough"
    );
    if let Ok(entries) = result {
        if let ConfigEntry::Rule(rule) = &entries[0] {
            assert!(rule.class.is_none());
            assert!(rule.title.is_none());
            assert!(rule.fallthrough);
        } else {
            panic!("Expected Rule entry");
        }
    }
}

#[test]
fn test_config_parses_matcherless_rule_without_fallthrough() {
    // A rule with no class/title and no fallthrough parses but will be rejected
    // by load_config validation (not tested here since it calls exit)
    let json = r#"[{"layer": "base"}]"#;
    let result: Result<Vec<ConfigEntry>, _> = serde_json::from_str(json);
    assert!(
        result.is_ok(),
        "Config should parse (validation happens in load_config)"
    );
    if let Ok(entries) = result {
        if let ConfigEntry::Rule(rule) = &entries[0] {
            assert!(rule.class.is_none());
            assert!(rule.title.is_none());
            assert!(!rule.fallthrough); // default is false
        } else {
            panic!("Expected Rule entry");
        }
    }
}

#[test]
fn test_config_parses_rule_with_class_no_fallthrough() {
    // A rule with a class matcher doesn't need fallthrough
    let json = r#"[{"class": "firefox", "layer": "browser"}]"#;
    let result: Result<Vec<ConfigEntry>, _> = serde_json::from_str(json);
    assert!(
        result.is_ok(),
        "Config should parse rule with class matcher"
    );
    if let Ok(entries) = result {
        if let ConfigEntry::Rule(rule) = &entries[0] {
            assert!(rule.class.is_some());
            assert!(!rule.fallthrough);
        } else {
            panic!("Expected Rule entry");
        }
    }
}
