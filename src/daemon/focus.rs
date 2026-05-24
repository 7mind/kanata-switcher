use crate::config::{NativeTerminalRule, Rule, WindowInfo, match_pattern};

// === Focus Handler ===

/// Individual action to execute on focus change
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FocusAction {
    /// Release a virtual key
    ReleaseVk(String),
    /// Switch to a layer
    ChangeLayer(String),
    /// Press and hold a virtual key (managed - will be released on next focus change)
    PressVk(String),
    /// Raw VK action (name, action: Press/Release/Tap/Toggle)
    RawVkAction(String, String),
}

/// Actions to execute on focus change, in order.
/// With fallthrough, all matching actions are collected and executed sequentially.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct FocusActions {
    /// Ordered list of actions to execute
    pub(crate) actions: Vec<FocusAction>,
    /// The new ordered list of managed VKs after execution (pressed top-to-bottom, released bottom-to-top)
    pub(crate) new_managed_vks: Vec<String>,
}

impl FocusActions {
    pub(crate) fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

pub(crate) const NATIVE_TERMINAL_RULE_INDEX: usize = usize::MAX;

#[derive(Debug)]
pub(crate) struct FocusHandler {
    pub(crate) rules: Vec<Rule>,
    pub(crate) native_terminal_rule: Option<NativeTerminalRule>,
    pub(crate) last_class: String,
    pub(crate) last_title: String,
    pub(crate) last_matched_rules: Vec<usize>,
    pub(crate) last_effective_layer: String,
    /// Currently held virtual keys, in order they were pressed (top-to-bottom rule order)
    pub(crate) current_virtual_keys: Vec<String>,
    pub(crate) quiet_focus: bool,
}

impl FocusHandler {
    pub(crate) fn new(
        rules: Vec<Rule>,
        native_terminal_rule: Option<NativeTerminalRule>,
        quiet_focus: bool,
    ) -> Self {
        Self {
            rules,
            native_terminal_rule,
            last_class: String::new(),
            last_title: String::new(),
            last_matched_rules: Vec::new(),
            last_effective_layer: String::new(),
            current_virtual_keys: Vec::new(),
            quiet_focus,
        }
    }

    /// Handle a focus change event. Returns actions to execute.
    /// With fallthrough, ALL matching actions are collected and executed in order.
    /// All matched virtual_keys are pressed and held simultaneously.
    pub(crate) fn handle(&mut self, win: &WindowInfo, default_layer: &str) -> Option<FocusActions> {
        let mut result = FocusActions::default();

        if win.is_native_terminal {
            return self.handle_native_terminal(default_layer);
        }

        // Handle unfocused state (no window has focus)
        if win.class.is_empty() && win.title.is_empty() {
            return self.handle_unfocused(default_layer);
        }

        if !self.quiet_focus {
            println!("[Focus] class=\"{}\" title=\"{}\"", win.class, win.title);
        }

        // Match rules with fallthrough support
        struct MatchedRule {
            index: usize,
            layer: Option<String>,
            virtual_key: Option<String>,
            raw_vk_actions: Vec<(String, String)>,
        }

        let mut matched_rules: Vec<MatchedRule> = Vec::new();

        for (index, rule) in self.rules.iter().enumerate() {
            if match_pattern(rule.class.as_deref(), &win.class)
                && match_pattern(rule.title.as_deref(), &win.title)
            {
                matched_rules.push(MatchedRule {
                    index,
                    layer: rule.layer.clone(),
                    virtual_key: rule.virtual_key.clone(),
                    raw_vk_actions: rule.raw_vk_action.clone().unwrap_or_default(),
                });

                if !rule.fallthrough {
                    break;
                }
            }
        }

        let matched_indices: Vec<usize> = matched_rules.iter().map(|rule| rule.index).collect();

        // Collect all VKs from matched rules in order (for holding)
        let new_vks: Vec<String> = matched_rules
            .iter()
            .filter_map(|r| r.virtual_key.clone())
            .collect();

        // Release VKs that are no longer matched (in reverse order)
        for vk in self.current_virtual_keys.iter().rev() {
            if !new_vks.contains(vk) {
                result.actions.push(FocusAction::ReleaseVk(vk.clone()));
            }
        }

        // If no rules matched, use default layer
        if matched_rules.is_empty() {
            if !default_layer.is_empty() && self.last_effective_layer != default_layer {
                result
                    .actions
                    .push(FocusAction::ChangeLayer(default_layer.to_string()));
            }
            result.new_managed_vks = Vec::new();
            self.last_effective_layer = default_layer.to_string();
        } else {
            let matched_changed = matched_indices != self.last_matched_rules;
            let mut matched_layers: Vec<String> = Vec::new();
            for matched in &matched_rules {
                if let Some(layer) = matched.layer.clone() {
                    matched_layers.push(layer);
                }
            }
            let new_rules: Vec<usize> = matched_indices
                .iter()
                .cloned()
                .filter(|idx| !self.last_matched_rules.contains(idx))
                .collect();

            // Process matched rules in order, building action list
            for matched in matched_rules {
                let is_new = new_rules.contains(&matched.index);
                if is_new {
                    // Layer change
                    if let Some(layer) = matched.layer {
                        result.actions.push(FocusAction::ChangeLayer(layer));
                    }

                    // Virtual key: press if not already held
                    if let Some(ref vk) = matched.virtual_key
                        && !self.current_virtual_keys.contains(vk)
                    {
                        result.actions.push(FocusAction::PressVk(vk.clone()));
                    }

                    // Raw VK actions
                    for (name, action) in matched.raw_vk_actions {
                        result.actions.push(FocusAction::RawVkAction(name, action));
                    }
                }
            }

            if matched_changed
                && let Some(new_layer) = matched_layers.last().cloned()
                && self.last_effective_layer != new_layer
            {
                let has_new_layer = result.actions.iter().rev().find_map(|action| {
                    if let FocusAction::ChangeLayer(layer) = action {
                        Some(layer == &new_layer)
                    } else {
                        None
                    }
                });
                if has_new_layer != Some(true) {
                    result
                        .actions
                        .push(FocusAction::ChangeLayer(new_layer.clone()));
                }
                self.last_effective_layer = new_layer;
            }

            result.new_managed_vks = new_vks;
        }

        // Update state
        self.last_class = win.class.clone();
        self.last_title = win.title.clone();
        self.last_matched_rules = matched_indices;
        self.current_virtual_keys = result.new_managed_vks.clone();

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    pub(crate) fn current_virtual_keys(&self) -> Vec<String> {
        self.current_virtual_keys.clone()
    }

    pub(crate) fn reset(&mut self) {
        self.last_class.clear();
        self.last_title.clear();
        self.last_matched_rules.clear();
        self.last_effective_layer.clear();
        self.current_virtual_keys.clear();
    }

    fn handle_unfocused(&mut self, default_layer: &str) -> Option<FocusActions> {
        let mut result = FocusActions::default();
        if !self.quiet_focus {
            println!("[Focus] No window focused");
        }
        // Release all active virtual keys in reverse order (bottom-to-top)
        for vk in self.current_virtual_keys.iter().rev() {
            result.actions.push(FocusAction::ReleaseVk(vk.clone()));
        }
        // Switch to default layer
        if !default_layer.is_empty() && self.last_effective_layer != default_layer {
            result
                .actions
                .push(FocusAction::ChangeLayer(default_layer.to_string()));
        }
        result.new_managed_vks = Vec::new();
        self.current_virtual_keys = Vec::new();
        self.last_matched_rules.clear();
        self.last_effective_layer = default_layer.to_string();
        self.last_class.clear();
        self.last_title.clear();
        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    fn handle_native_terminal(&mut self, default_layer: &str) -> Option<FocusActions> {
        let Some(rule) = self.native_terminal_rule.clone() else {
            return self.handle_native_terminal_without_rule(default_layer);
        };

        if !self.quiet_focus {
            println!("[Focus] Native terminal active");
        }

        let mut result = FocusActions::default();
        let mut new_vks = Vec::new();

        if let Some(vk) = rule.virtual_key.clone() {
            new_vks.push(vk);
        }

        for vk in self.current_virtual_keys.iter().rev() {
            if !new_vks.contains(vk) {
                result.actions.push(FocusAction::ReleaseVk(vk.clone()));
            }
        }

        let matched_indices = vec![NATIVE_TERMINAL_RULE_INDEX];
        let is_new = self.last_matched_rules != matched_indices;

        if is_new {
            if !rule.layer.is_empty() && self.last_effective_layer != rule.layer {
                result
                    .actions
                    .push(FocusAction::ChangeLayer(rule.layer.clone()));
            }
            if let Some(vk) = rule.virtual_key
                && !self.current_virtual_keys.contains(&vk)
            {
                result.actions.push(FocusAction::PressVk(vk));
            }
            for (name, action) in rule.raw_vk_action {
                result.actions.push(FocusAction::RawVkAction(name, action));
            }
        }

        result.new_managed_vks = new_vks;
        self.last_matched_rules = matched_indices;
        self.last_effective_layer = rule.layer;
        self.current_virtual_keys = result.new_managed_vks.clone();
        self.last_class.clear();
        self.last_title.clear();

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    fn handle_native_terminal_without_rule(&mut self, default_layer: &str) -> Option<FocusActions> {
        let mut result = FocusActions::default();
        if !self.quiet_focus {
            println!("[Focus] Native terminal active");
        }
        for vk in self.current_virtual_keys.iter().rev() {
            result.actions.push(FocusAction::ReleaseVk(vk.clone()));
        }
        if !default_layer.is_empty() && self.last_effective_layer != default_layer {
            result
                .actions
                .push(FocusAction::ChangeLayer(default_layer.to_string()));
        }
        result.new_managed_vks = Vec::new();
        self.current_virtual_keys = Vec::new();
        self.last_matched_rules.clear();
        self.last_effective_layer = default_layer.to_string();
        self.last_class.clear();
        self.last_title.clear();

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }
}
