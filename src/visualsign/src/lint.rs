use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Ok,
    Warn,
    Error,
    Allow,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Ok => "ok",
            Severity::Warn => "warn",
            Severity::Error => "error",
            Severity::Allow => "allow",
        }
    }
}

/// Configuration for lint rule behavior.
///
/// Controls which rules run, their default severity, and whether
/// ok-level diagnostics are emitted (boot metrics mode).
#[derive(Debug, Clone)]
pub struct LintConfig {
    /// Override severity for specific rules. Key is the rule ID
    /// (e.g., "transaction::oob_program_id").
    pub overrides: HashMap<String, Severity>,

    /// When true, rules that find no issues emit an ok-level diagnostic.
    /// This provides boot-metric-style attestation where the verifier
    /// can confirm every expected rule ran.
    pub report_all_rules: bool,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self {
            overrides: HashMap::new(),
            report_all_rules: true,
        }
    }
}

impl LintConfig {
    /// Get the effective severity for a rule, falling back to the provided default.
    pub fn severity_for(&self, rule: &str, default: Severity) -> Severity {
        self.overrides.get(rule).cloned().unwrap_or(default)
    }

    /// Whether an ok-level diagnostic should be emitted for this rule.
    pub fn should_report_ok(&self, rule: &str) -> bool {
        if !self.report_all_rules {
            return false;
        }
        // If the rule is explicitly set to Allow, don't emit ok either
        if let Some(Severity::Allow) = self.overrides.get(rule) {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_emits_ok() {
        let config = LintConfig::default();
        assert!(config.report_all_rules);
        assert!(config.should_report_ok("transaction::oob_program_id"));
    }

    #[test]
    fn test_severity_override() {
        let mut config = LintConfig::default();
        config
            .overrides
            .insert("transaction::oob_program_id".to_string(), Severity::Error);
        assert!(matches!(
            config.severity_for("transaction::oob_program_id", Severity::Warn),
            Severity::Error
        ));
        assert!(matches!(
            config.severity_for("transaction::oob_account_index", Severity::Warn),
            Severity::Warn
        ));
    }

    #[test]
    fn test_allow_suppresses_ok() {
        let mut config = LintConfig::default();
        config
            .overrides
            .insert("transaction::oob_program_id".to_string(), Severity::Allow);
        assert!(!config.should_report_ok("transaction::oob_program_id"));
        assert!(config.should_report_ok("transaction::oob_account_index"));
    }

    #[test]
    fn test_disable_ok_diagnostics() {
        let config = LintConfig {
            report_all_rules: false,
            ..LintConfig::default()
        };
        assert!(!config.should_report_ok("transaction::oob_program_id"));
    }
}
