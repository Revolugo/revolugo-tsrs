//! The options of a ported rule.

use tsrs_core::json::Value;

/// The options of the rule `name` as `ESLint` numbers them: `[]` when the payload gives none.
pub fn of<'a>(name: &str, options: &'a Value) -> Result<&'a [Value], String> {
    match options {
        Value::Null => Ok(&[]),
        Value::Array(options) => Ok(options),
        _ => {
            Err(format!("the options of {name} are an array, as in the configuration of the rule"))
        }
    }
}
