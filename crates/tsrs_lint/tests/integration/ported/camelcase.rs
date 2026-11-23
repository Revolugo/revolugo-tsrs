//! `js/camelcase`: what the rule reports, case by case. The cases are those of the JS rule's fixtures
//! (`revolugo-lint-plugins`, `test/core/fixtures/camelcase`) and of each branch of its source.

use tsrs_core::json;
use tsrs_lint::payload::ConfiguredRule;

use super::{lint, project};

/// What the rule marks in `source`, written as `file` in a project of its own, in the order of the file.
fn marked(test: &str, file: &str, source: &str, options: &str) -> Vec<String> {
    let base = project(&format!("camelcase-{test}"), &[(file, source)]);
    lint(&base, file, "js/camelcase", options).into_iter().map(|(text, _)| text).collect()
}

/// The error that the rule gives for `options`.
fn refusal(options: &str) -> String {
    let options = json::unmarshal(options).unwrap();
    let config = ConfiguredRule { name: "js/camelcase".into(), options };
    match tsrs_lint::ported::create_rule(&config) {
        Some(Err(error)) => error,
        Some(Ok(_)) => panic!("the options are accepted"),
        None => panic!("js/camelcase is not a ported rule"),
    }
}

#[test]
fn a_name_with_an_underscore_inside_is_reported() {
    let source =
        "first_name = firstName = FIRST_NAME = _leading = trailing_ = __a_b__ = __both__\n";
    let base = project("camelcase-names", &[("a.js", source)]);
    assert_eq!(
        lint(&base, "a.js", "js/camelcase", "[]"),
        [
            ("first_name".to_string(), "Identifier 'first_name' is not in camel case.".to_string()),
            ("__a_b__".to_string(), "Identifier '__a_b__' is not in camel case.".to_string()),
        ]
    );
}

#[test]
fn a_name_is_upper_case_as_javascript_upper_cases_it() {
    let source = "x = ÀB_CD + àb_cd + A_ß + A1_2 + a1_2\n";
    assert_eq!(marked("upper-case", "a.js", source, "[]"), ["àb_cd", "A_ß", "a1_2"]);
}

#[test]
fn a_private_name_is_reported_with_its_hash() {
    let source =
        "class A {\n  #private_field = 1\n  #private_method() {}\n  #okName = 2\n  #_x = 3\n}\n";
    let base = project("camelcase-private", &[("a.js", source)]);
    assert_eq!(
        lint(&base, "a.js", "js/camelcase", "[]"),
        [
            ("#private_field".to_string(), "#private_field is not in camel case.".to_string()),
            ("#private_method".to_string(), "#private_method is not in camel case.".to_string()),
        ]
    );
}

#[test]
fn variables_functions_classes_and_parameters_are_reported() {
    let source = "export {}
var my_var = 1
let { in_pattern, renamed: to_name, ...rest_props } = source
function my_fn(my_param, [in_array] = [], ...rest_params) {}
class My_class {}
const my_arrow = (arrow_param) => 1
const fn = function named_fn() {}
const cls = class Named_class {}
try {} catch (caught_error) {}
";
    assert_eq!(
        marked("declarations", "a.js", source, "[]"),
        [
            "my_var",
            "in_pattern",
            "to_name",
            "rest_props",
            "my_fn",
            "my_param",
            "in_array",
            "rest_params",
            "My_class",
            "my_arrow",
            "arrow_param",
            "named_fn",
            "Named_class",
            "caught_error",
        ]
    );
}

#[test]
fn the_references_to_a_reported_variable_are_reported() {
    let source = "export {}
var my_var = 1
my_var = 2
my_var++
x = my_var.y + typeof my_var
function f(my_param) {
  return my_param
}
";
    assert_eq!(
        marked("references", "a.js", source, "[]"),
        ["my_var", "my_var", "my_var", "my_var", "my_var", "my_param", "my_param"]
    );
}

#[test]
fn a_reference_that_no_declaration_resolves_is_reported() {
    let source = "x = undeclared_read\nundeclared_write = 1\ntagged_template`a${in_template}`\n";
    assert_eq!(
        marked("undeclared", "a.js", source, "[]"),
        ["undeclared_read", "undeclared_write", "tagged_template", "in_template"]
    );
}

#[test]
fn a_call_a_new_and_a_default_value_keep_their_references_unreported() {
    let source = "export {}
var my_fn, my_arg, My_ctor, my_default
my_fn(my_arg, (my_arg))
new My_ctor(my_arg)
function f(a = my_default, { b = my_default }) {}
;[c = my_default] = d
;({ e = my_default, f: g = my_default } = d)
import(my_arg)
my_fn(my_arg.h, ...my_arg)
";
    assert_eq!(
        marked("calls", "a.js", source, "[]"),
        ["my_fn", "my_arg", "My_ctor", "my_default", "my_arg", "my_arg", "my_arg"]
    );
}

#[test]
fn the_first_declaration_of_a_variable_is_the_one_reported() {
    let source = "export {}
var twice_var = 1
var twice_var = 2
function over_fn(a: string): void
function over_fn(a: number): void
function over_fn(a: unknown) {}
type Type_first = string
const Type_first = 1
";
    assert_eq!(
        marked("first-declaration", "a.ts", source, "[]"),
        ["twice_var", "over_fn", "Type_first"]
    );
}

#[test]
fn the_name_of_a_typed_binding_is_reported_with_its_type() {
    let source = "export {}
function f(typed_param: string, opt_param?: number, bare_opt?, def_param: number = 1, ...rest_param: string[]) {}
let typed_let: string, definite_let!: number
";
    assert_eq!(
        marked("typed-bindings", "a.ts", source, "[]"),
        [
            "typed_param: string",
            "opt_param?: number",
            "bare_opt?",
            "def_param: number",
            "rest_param",
            "typed_let: string",
            "definite_let!: number",
        ]
    );
}

#[test]
fn an_import_is_reported_with_its_references() {
    let source = "import { no_camelcased } from 'm'
import { no_camelcased as camelCased, other as another_two } from 'm'
import * as name_space from 'm'
import default_import from 'm'
import type { Type_import } from 'm'
let a: Type_import = name_space.b
";
    assert_eq!(
        marked("imports", "a.ts", source, "[]"),
        [
            "no_camelcased",
            "another_two",
            "name_space",
            "default_import",
            "Type_import",
            "Type_import",
            "name_space",
        ]
    );
}

#[test]
fn what_typescript_alone_declares_is_not_reported() {
    let source = "export {}
interface Some_interface<Type_param> {
  prop_name: Type_param
  method_name(method_param: string): typeof method_param
  [index_key: string]: unknown
}
type Type_alias = { [Key_name in keyof Some_interface<string>]: Key_name }
enum Enum_name {
  member_name = 1,
  'string_member' = 2,
  other_member = member_name + string_member,
}
namespace Name_space {}
declare function declared_fn(declared_param: string): void
import required_module = require('m')
let a: Some_interface<Type_alias> = [Enum_name.member_name, Name_space, declared_fn, required_module]
";
    assert_eq!(marked("typescript-only", "a.ts", source, "[]"), [] as [&str; 0]);
}

#[test]
fn a_function_or_a_method_without_body_does_not_report_its_parameters() {
    let source = "export {}
function over_fn(over_param: string): void
function over_fn(over_param: unknown) {}
abstract class A {
  abstract abstractMethod(abstract_param: string): void
  overloaded(first_param: string): void
  overloaded(first_param: unknown) {}
}
";
    assert_eq!(
        marked("without-body", "a.ts", source, "[]"),
        ["over_fn", "over_param: unknown", "first_param: unknown"]
    );
}

#[test]
fn a_declaration_is_visible_in_its_block_only() {
    let source = "export {}
{
  enum In_block {}
  In_block
}
In_block
for (let in_for of list) {}
in_for
";
    assert_eq!(marked("blocks", "a.ts", source, "[]"), ["In_block", "in_for", "in_for"]);
}

#[test]
fn a_var_is_visible_in_its_whole_function() {
    let source = "export {}
function f() {
  {
    enum hoisted_name {}
  }
  if (a) {
    var hoisted_name = 1
  }
  return hoisted_name
}
";
    // The reference resolves to the variable, not to the enum of the inner block: it is reported as the variable is.
    assert_eq!(marked("var", "a.ts", source, "[]"), ["hoisted_name", "hoisted_name"]);
}

#[test]
fn a_parameter_does_not_see_what_the_body_declares() {
    let source = "export {}
function f(a = () => Body_enum) {
  enum Body_enum {}
  return Body_enum
}
";
    assert_eq!(marked("parameters", "a.ts", source, "[]"), ["Body_enum"]);
}

#[test]
fn a_type_and_a_value_resolve_to_declarations_of_their_kind() {
    let source = "export {}
type Type_only = string
const value_only = 1
let a: value_only = Type_only
let b: Type_only = value_only
";
    // In `a`, neither name resolves: both are reported. In `b`, the type resolves to the alias, which is not
    // reported, and the value to the constant.
    assert_eq!(
        marked("kinds", "a.ts", source, "[]"),
        ["value_only", "value_only", "Type_only", "value_only"]
    );
}

#[test]
fn a_script_resolves_a_reference_to_any_declaration_of_the_file() {
    let source = "type Type_only = string\nlet a = Type_only\n";
    assert_eq!(marked("script", "a.ts", source, "[]"), [] as [&str; 0]);
    for (test, module) in [
        ("module-export", "export {}\n"),
        ("module-meta", "import.meta\n"),
        ("module-await", "await a\n"),
    ] {
        let source = format!("{source}{module}");
        assert_eq!(marked(test, "a.ts", &source, "[]"), ["Type_only"], "{module}");
    }
    assert_eq!(marked("module-extension", "a.mts", source, "[]"), ["Type_only"]);
    assert_eq!(marked("commonjs-extension", "a.cts", source, "[]"), ["Type_only"]);
}

#[test]
fn a_dotted_namespace_declares_no_name() {
    let source = "export {}
namespace Single_name {}
namespace Dotted_name.Inner_name {}
x = [Single_name, Dotted_name]
";
    assert_eq!(marked("namespaces", "a.ts", source, "[]"), ["Dotted_name"]);
}

#[test]
fn the_false_branch_of_a_conditional_type_does_not_see_what_it_infers() {
    let source =
        "export type A<T> = T extends (infer In_condition)[] ? In_condition : In_condition\n";
    assert_eq!(marked("infer", "a.ts", source, "[]"), ["In_condition"]);
}

#[test]
fn the_keys_of_objects_and_of_class_members_are_reported() {
    let source = "export {}
x = {
  key_name: 1,
  'quoted_key': 2,
  [computed_key]: 3,
  short_hand,
  method_key() {},
  get getter_key() {
    return 1
  },
}
class A {
  field_name = 1
  static static_method() {}
  ['computed_member']() {}
}
";
    assert_eq!(
        marked("keys", "a.js", source, "[]"),
        [
            "key_name",
            "computed_key",
            "short_hand",
            "method_key",
            "getter_key",
            "field_name",
            "static_method",
        ]
    );
}

#[test]
fn the_members_of_typescript_that_are_not_definitions_are_not_reported() {
    let source = "export {}
abstract class A {
  abstract abstract_field: string
  abstract abstract_method(): void
  accessor accessor_field = 1
  declare declared_field: string
  optional_field?: string
}
";
    assert_eq!(marked("ts-members", "a.ts", source, "[]"), ["declared_field", "optional_field"]);
}

#[test]
fn a_member_is_reported_where_it_is_assigned() {
    let source = "export {}
obj.prop_name = 1
obj.compound_member += 1
obj.logical_member ??= 1
;(obj.in_parentheses) = 1
;[obj.in_array, obj.with_default = 1, ...obj.rest_element] = source
;({ key: obj.in_object, ...obj.rest_property } = source)
obj.read_only
obj.updated_member++
delete obj.deleted_member
for (obj.loop_member of source) {
}
obj.first_member.second_member = obj.right_member
";
    assert_eq!(
        marked("members", "a.js", source, "[]"),
        [
            "prop_name",
            "compound_member",
            "logical_member",
            "in_parentheses",
            "in_array",
            "with_default",
            "rest_element",
            "in_object",
            "rest_property",
            "second_member",
        ]
    );
}

#[test]
fn the_keys_of_a_destructuring_are_not_reported() {
    let source = "export {}
let { key_name: a } = source
;({ other_key: b, nested_key: { deep_key: c } } = source)
for ({ loop_key: d } of source) {
}
";
    assert_eq!(marked("pattern-keys", "a.js", source, "[]"), [] as [&str; 0]);
}

#[test]
fn the_options_of_a_dynamic_import_are_not_reported() {
    let source = "export {}
import('m', { with: { some_attr: 'json', nested: { deep_attr: 1 } }, [computed]: { reported_key: 1 } })
other('m', { reported_too: 1 })
";
    assert_eq!(marked("import-options", "a.js", source, "[]"), ["reported_key", "reported_too"]);
}

#[test]
fn properties_never_keeps_keys_and_members_unreported() {
    let source = "export {}
x = { key_name: 1, short_hand }
class A {
  field_name = 1
  #private_field = 2
}
obj.prop_name = 1
";
    // The value of a shorthand property is a reference.
    assert_eq!(
        marked("properties-never", "a.js", source, r#"[{ "properties": "never" }]"#),
        ["short_hand"]
    );
}

#[test]
fn exported_names_are_reported_whatever_the_properties_option() {
    let source = "const a = 1
export { a as exported_name, a as okName }
export * as star_export from 'm'
export { reexport_name, other as renamed_export, from_name as okOther } from 'm'
";
    assert_eq!(
        marked("exports", "a.js", source, r#"[{ "properties": "never" }]"#),
        ["exported_name", "star_export", "reexport_name", "renamed_export"]
    );
}

#[test]
fn an_exported_name_is_a_reference_to_a_value_or_to_a_type() {
    let source = "interface Some_interface {}
export { Some_interface as a, undeclared_name as b }
export default Some_interface
";
    assert_eq!(marked("export-references", "a.ts", source, "[]"), ["undeclared_name"]);
}

#[test]
fn labels_are_reported() {
    let source = "label_name: for (;;) {
  if (a) {
    break label_name
  }
  continue label_name
}
";
    assert_eq!(marked("labels", "a.js", source, "[]"), ["label_name", "label_name", "label_name"]);
}

#[test]
fn a_jsx_element_is_a_reference_unless_its_name_is_lower_case() {
    let source = "export const a = (
  <Custom_list snake_attr=\"b\">
    <lower_case />
    <_under_scored />
    <Name_space.member_name />
  </Custom_list>
)
";
    assert_eq!(
        marked("jsx", "a.tsx", source, "[]"),
        ["Custom_list", "_under_scored", "Name_space", "Custom_list"]
    );
}

#[test]
fn the_types_of_a_jsdoc_comment_are_not_read() {
    let source = "export {}
/** @typedef {object} typedef_name */
/** @type {typedef_name} */
var a = /** @type {cast_type} */ (b)
/** @param {param_type} c */
function f(c) {
  g(/** @type {cast_type} */ (call_argument))
}
";
    assert_eq!(marked("jsdoc", "a.js", source, "[]"), [] as [&str; 0]);
}

#[test]
fn allow_takes_names_and_regular_expressions() {
    let source = "export {}
var plain_name, date_from, date_to, date_until, count_one, some_long_other, not_allowed
";
    let options = r#"[{ "allow": ["plain_name", "^date_(from|to)$", "_(one|other)$"] }]"#;
    assert_eq!(marked("allow", "a.js", source, options), ["date_until", "not_allowed"]);
}

#[test]
fn an_allow_entry_is_a_regular_expression_of_javascript_with_the_u_flag() {
    let source = "export {}\nvar été_chaud, ete_chaud, a_b\n";
    let options = r#"[{ "allow": ["^\\p{L}+_chaud$", "^(?<first>a)_(?!c)b$"] }]"#;
    assert_eq!(marked("allow-unicode", "a.js", source, options), [] as [&str; 0]);
}

#[test]
fn ignore_destructuring_spares_the_names_that_a_pattern_keeps() {
    let source = "export {}
let { a_b } = source
let { c_d = 1 } = source
let { e_f: e_f } = source
let { g: h_i } = source
let [j_k] = source
x = a_b + h_i
y = { a_b }
";
    // `properties` is "never", so that the key of `{ a_b }` is not what is asked about.
    let options = r#"[{ "ignoreDestructuring": true, "properties": "never" }]"#;
    assert_eq!(
        marked("ignore-destructuring", "a.js", source, options),
        ["h_i", "j_k", "a_b", "h_i"]
    );
}

#[test]
fn ignore_imports_spares_the_names_that_an_import_keeps() {
    let source = "import { a_b, c as d_e } from 'm'
import f_g from 'm'
x = a_b
";
    assert_eq!(
        marked("ignore-imports", "a.js", source, r#"[{ "ignoreImports": true }]"#),
        ["d_e", "f_g", "a_b"]
    );
}

#[test]
fn ignore_globals_is_accepted() {
    let source = "x = undeclared_global\n";
    assert_eq!(
        marked("ignore-globals", "a.js", source, r#"[{ "ignoreGlobals": true }]"#),
        ["undeclared_global"]
    );
}

#[test]
fn options_that_the_schema_of_the_rule_refuses_are_errors() {
    for (options, error) in [
        (r#"["always"]"#, "js/camelcase: the options are an object"),
        (r"[{}, {}]", "js/camelcase: takes one options object"),
        (r#"[{ "allow": "a_b" }]"#, "js/camelcase: allow is an array of strings"),
        (r#"[{ "allow": [1] }]"#, "js/camelcase: allow is an array of strings"),
        (r#"[{ "allow": ["a_b", "a_b"] }]"#, "js/camelcase: allow has \"a_b\" twice"),
        (r#"[{ "ignoreDestructuring": 1 }]"#, "js/camelcase: ignoreDestructuring is a boolean"),
        (r#"[{ "ignoreImports": "yes" }]"#, "js/camelcase: ignoreImports is a boolean"),
        (r#"[{ "ignoreGlobals": null }]"#, "js/camelcase: ignoreGlobals is a boolean"),
        (
            r#"[{ "properties": "sometimes" }]"#,
            "js/camelcase: properties is \"always\" or \"never\"",
        ),
        (r#"[{ "property": "never" }]"#, "js/camelcase: unknown option \"property\""),
    ] {
        assert_eq!(refusal(options), error, "{options}");
    }
}

#[test]
fn an_allow_entry_that_is_not_a_regular_expression_is_an_error() {
    let error = refusal(r#"[{ "allow": ["(unclosed"] }]"#);
    assert!(error.starts_with("js/camelcase: invalid pattern \"(unclosed\" in allow: "), "{error}");
}
