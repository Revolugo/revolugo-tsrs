use std::process::Command;

use tsrs_lint::payload::ConfiguredRule;
use tsrs_lint::registry;

#[test]
fn rules_lists_every_rule_a_payload_can_name_one_per_line() {
    let output = Command::new(env!("CARGO_BIN_EXE_tsrs-lint")).arg("rules").output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.ends_with('\n'), "{text:?}");
    let listed: Vec<&str> = text.lines().collect();

    assert!(listed.is_sorted(), "{listed:?}");
    assert!(listed.contains(&"no-floating-promises"), "{listed:?}");
    // The inventory, less the rules that are not implemented: a client can send every listed rule, and no other.
    for name in registry::BASELINE_RULES {
        let implemented = registry::require_implemented(name).is_ok();
        assert_eq!(listed.contains(name), implemented, "{name}");
    }
    assert_eq!(listed.len(), registry::implemented_rules().len());
    for name in &listed {
        let rule = ConfiguredRule { name: (*name).to_string(), ..Default::default() };
        assert!(registry::create_rule(&rule).is_ok(), "{name}");
    }
}

#[test]
fn rules_needs_no_payload_and_headless_still_requires_one_of_the_two_commands() {
    let listed = tsrs_lint::cli::run(["rules".to_string()], &mut std::io::empty(), &mut Vec::new());
    assert_eq!(listed, 0);

    let output = Command::new(env!("CARGO_BIN_EXE_tsrs-lint")).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8(output.stderr).unwrap(), "tsrs-lint requires headless or rules\n");
}
