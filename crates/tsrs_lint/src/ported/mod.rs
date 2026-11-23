//! Rules that are not typescript-eslint's (revolugo fork).
//!
//! Each one is the port of a rule of the `js` plugin of `revolugo-lint-plugins`, which is itself a port of a rule of
//! `ESLint` or of one of its plugins. Oxlint runs the rules of a JS plugin on one thread. Here they run on the
//! checker threads, with the rules of typescript-eslint.
//!
//! A payload names a ported rule with its plugin, `js/camelcase`, and gives it the options of the rule as `ESLint`
//! takes them: the array of what follows the severity in the configuration, or nothing. Oxlint asks for none of
//! them: the lint command of the npm package does (`docs/LINT.md`).

use crate::payload::ConfiguredRule;
use crate::rule::Rule;

mod camelcase;
mod import_export;
mod no_invalid_this;
mod options;

/// The ported rules, in name order.
pub const RULES: &[&str] = &["js/camelcase", "js/import-export", "js/no-invalid-this"];

/// The rule that `config` names, or `None` when it names no ported rule.
pub fn create_rule(config: &ConfiguredRule) -> Option<Result<Box<dyn Rule>, String>> {
    if !RULES.contains(&config.name.as_str()) {
        return None;
    }
    let options = match options::of(&config.name, &config.options) {
        Ok(options) => options,
        Err(error) => return Some(Err(error)),
    };
    Some(match config.name.as_str() {
        "js/camelcase" => {
            camelcase::Camelcase::new(options).map(|rule| Box::new(rule) as Box<dyn Rule>)
        }
        "js/import-export" => {
            import_export::ImportExport::new(options).map(|rule| Box::new(rule) as Box<dyn Rule>)
        }
        "js/no-invalid-this" => {
            no_invalid_this::NoInvalidThis::new(options).map(|rule| Box::new(rule) as Box<dyn Rule>)
        }
        name => unreachable!("{name} is in RULES"),
    })
}
