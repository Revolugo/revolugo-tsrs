//! Tests of the port of `js/no-invalid-this`. Each case is a file of its own, linted through the headless command:
//! the extension and the content of a file decide whether it is a module, a script or a `CommonJS` file.

use std::sync::atomic::{AtomicUsize, Ordering};

use tsrs_core::json;
use tsrs_lint::payload::ConfiguredRule;

use super::{lint, project};

const RULE: &str = "js/no-invalid-this";
const DEFAULT: &str = "[]";
const NO_CONSTRUCTORS: &str = r#"[{"capIsConstructor":false}]"#;

/// How many `this` the rule reports in the file `file` with the content `text`.
fn reported(options: &str, file: &str, text: &str) -> usize {
    static PROJECTS: AtomicUsize = AtomicUsize::new(0);
    let name = format!("no-invalid-this-{}", PROJECTS.fetch_add(1, Ordering::Relaxed));
    let base = project(&name, &[(file, text)]);
    let reported = lint(&base, file, RULE, options);
    for (marked, message) in &reported {
        assert_eq!((marked.as_str(), message.as_str()), ("this", "Unexpected 'this'."), "{text}");
    }
    reported.len()
}

/// Each of `cases` is a file named `file` with one `this` that the rule reports.
#[track_caller]
fn assert_reported(options: &str, file: &str, cases: &[&str]) {
    for case in cases {
        assert_eq!(reported(options, file, case), 1, "in {file}: {case}");
    }
}

/// Each of `cases` is a file named `file` where the rule reports nothing.
#[track_caller]
fn assert_valid(options: &str, file: &str, cases: &[&str]) {
    for case in cases {
        assert_eq!(reported(options, file, case), 0, "in {file}: {case}");
    }
}

fn options_error(options: &str) -> Option<String> {
    let config = ConfiguredRule { name: RULE.into(), options: json::unmarshal(options).unwrap() };
    tsrs_lint::ported::create_rule(&config).expect("a ported rule").err()
}

#[test]
fn the_report_marks_this_with_the_message_of_eslint() {
    let base = project("no-invalid-this-report", &[("a.mjs", "console.log(this)\n")]);
    assert_eq!(
        lint(&base, "a.mjs", RULE, DEFAULT),
        vec![("this".to_string(), "Unexpected 'this'.".to_string())]
    );
}

// --- The top level of a file

#[test]
fn this_at_the_top_level_of_a_module_is_reported() {
    for file in ["a.mjs", "a.mts"] {
        assert_reported(DEFAULT, file, &["console.log(this)\n", "const arrow = () => this\n"]);
    }
    assert_reported(DEFAULT, "a.js", &["export {}\nconsole.log(this)\n"]);
    assert_reported(
        DEFAULT,
        "a.ts",
        &["import type { A } from './b'\nexport const top: A = this\n"],
    );
}

#[test]
fn this_at_the_top_level_of_a_script_is_valid() {
    for file in ["a.js", "a.jsx", "a.ts", "a.tsx"] {
        assert_valid(
            DEFAULT,
            file,
            &["var top = this\nvar arrow = () => this\n", "'use strict'\nvar top = this\n"],
        );
    }
}

#[test]
fn a_file_with_an_import_or_an_export_statement_is_a_module() {
    assert_reported(
        DEFAULT,
        "a.js",
        &[
            "import './b'\nvar top = this\n",
            "export default 1\nvar top = this\n",
            "export * from './b'\nvar top = this\n",
            "var top = this\nexport { top }\n",
            "export function exported() {}\nvar top = this\n",
        ],
    );
    assert_reported(
        DEFAULT,
        "a.ts",
        &[
            "export type A = 1\nvar top = this\n",
            "var top = this\nexport = top\n",
            "export as namespace Library\nvar top = this\n",
            "namespace N { export const a = 1 }\nexport import a = N.a\nvar top = this\n",
        ],
    );
}

#[test]
fn import_meta_and_an_await_outside_of_a_function_make_a_module() {
    assert_reported(
        DEFAULT,
        "a.js",
        &[
            "var url = import.meta.url\nvar top = this\n",
            "function inner() { return import.meta.url }\nvar top = this\n",
            "var value = await load()\nvar top = this\n",
            "if (value) { await load() }\nvar top = this\n",
            "class A { field = await load() }\nvar top = this\n",
        ],
    );
}

#[test]
fn what_looks_like_esm_syntax_and_is_not_leaves_a_script() {
    assert_valid(
        DEFAULT,
        "a.ts",
        &[
            "import fs = require('fs')\nvar top = this\n",
            "namespace N { export const a = 1 }\nvar top = this\n",
            "declare module 'b' { export const a: number }\nvar top = this\n",
            "declare global { var value: number }\nvar top = this\n",
        ],
    );
    assert_valid(
        DEFAULT,
        "a.js",
        &[
            "async function load() { await other() }\nvar top = this\n",
            "var load = async () => await other()\nvar top = this\n",
            "var lazy = import('./b')\nvar top = this\n",
            "// await import.meta\nvar top = this\n",
            "/** @import { A } from './b' */\nvar top = this\n",
            "var b = require('./b')\nmodule.exports = b\nvar top = this\n",
        ],
    );
}

#[test]
fn this_at_the_top_level_of_a_commonjs_file_is_reported_when_the_file_is_strict() {
    for file in ["a.cjs", "a.cts"] {
        assert_reported(DEFAULT, file, &["'use strict'\nthis.exported = 1\n"]);
        assert_valid(DEFAULT, file, &["this.exported = 1\n", "const arrow = () => this\n"]);
    }
}

// --- Strict mode

#[test]
fn this_in_a_function_that_is_not_strict_is_valid() {
    for file in ["a.js", "a.ts", "a.cjs", "a.cts"] {
        assert_valid(
            DEFAULT,
            file,
            &[
                "function sloppy() { return this }\n",
                "var sloppy = function () { return function () { return this } }\n",
            ],
        );
    }
}

#[test]
fn a_use_strict_directive_makes_a_function_and_the_functions_in_it_strict() {
    assert_reported(
        DEFAULT,
        "a.js",
        &[
            "function strict() { 'use strict'; return this }\n",
            "function strict() { \"use strict\"; return this }\n",
            "function second() { 'use asm'; 'use strict'; return this }\n",
            "function outer() { 'use strict'; return function () { return this } }\n",
            "var arrow = () => { 'use strict'; return function () { return this } }\n",
            "var object = { method() { 'use strict'; return function () { return this } } }\n",
            "'use strict'\nfunction strict() { return this }\n",
            "// comment\n'use strict'\nfunction strict() { return this }\n",
        ],
    );
}

#[test]
fn a_string_that_is_not_a_use_strict_directive_leaves_a_function_sloppy() {
    assert_valid(
        DEFAULT,
        "a.js",
        &[
            "function parenthesized() { ('use strict'); return this }\n",
            "function late() { var a = 1; 'use strict'; return this }\n",
            "function escaped() { 'use\\x20strict'; return this }\n",
            "function template() { `use strict`; return this }\n",
            "function member() { 'use strict'.length; return this }\n",
            "var a = 1\n'use strict'\nfunction late() { return this }\n",
        ],
    );
}

#[test]
fn the_code_of_a_class_a_namespace_and_an_enum_is_strict() {
    assert_reported(
        DEFAULT,
        "a.ts",
        &[
            "class A { method() { return function () { return this } } }\n",
            "var A = class { static { var inner = function () { return this } } }\n",
            "class A extends (function () { return this } as any) {}\n",
            "namespace N { function inner() { return this } }\n",
            "enum E { A = (function () { return this })() as any }\n",
        ],
    );
}

#[test]
fn the_decorators_of_a_class_are_outside_of_its_strict_code() {
    assert_valid(DEFAULT, "a.ts", &["@decorate(function () { return this }) class A {}\n"]);
    assert_reported(
        DEFAULT,
        "a.ts",
        &["class A { @decorate(function () { return this }) method() {} }\n"],
    );
}

// --- Functions

#[test]
fn this_in_a_function_that_nothing_calls_with_a_this_is_reported() {
    assert_reported(
        DEFAULT,
        "a.mjs",
        &[
            "function declaration() { this.a = 0 }\n",
            "const expression = function () { return this.a }\n",
            "async function asynchronous() { await this.load() }\n",
            "function* generator() { yield this }\n",
            "export default function () { return this }\n",
            "use(function () { return this })\n",
            ";(function () { return this })()\n",
            "new (function () { this.a = 1 })()\n",
            "function defaults(a = this) { return a }\n",
        ],
    );
}

#[test]
fn an_arrow_function_has_the_this_of_the_code_around_it() {
    assert_reported(
        DEFAULT,
        "a.mjs",
        &["function outer() { return () => () => this }\n", "const top = { arrow: () => this }\n"],
    );
    assert_valid(
        DEFAULT,
        "a.mjs",
        &[
            "const object = { method() { return () => () => this } }\n",
            "class A { field = () => this }\n",
        ],
    );
}

#[test]
fn this_in_a_method_is_valid() {
    assert_valid(
        DEFAULT,
        "a.mjs",
        &[
            "const object = { method() { return this } }\n",
            "const object = { property: function () { return this } }\n",
            "const object = { get value() { return this }, set value(v) { this.v = v } }\n",
            "const object = { async *['computed'](a = this) { yield this } }\n",
            "class A { constructor(a = this) { this.a = a } }\n",
            "class A { method() { return this } static method() { return this } }\n",
            "class A { get value() { return this } set value(v) { this.v = v } }\n",
        ],
    );
}

#[test]
fn this_in_a_function_in_a_method_is_reported() {
    assert_reported(
        DEFAULT,
        "a.mjs",
        &[
            "const object = { method() { function inner() { return this } return inner } }\n",
            "class A { method() { return [function () { return this }, this] } }\n",
            "class A { get value() { return function () { return this } } }\n",
        ],
    );
}

#[test]
fn this_in_a_class_field_and_in_a_static_block_is_valid() {
    assert_valid(
        DEFAULT,
        "a.mts",
        &[
            "class A { field = this.a; static field = this; #secret = this }\n",
            "class A { field = function () { return this } }\n",
            "class A { field = cond ? function () { return this } : null }\n",
            "class A { field = function () { return this } as Handler }\n",
            "class A { field = { method() { return this }, arrow: () => this } }\n",
            "class A { static { this.a = 1; const arrow = () => this } }\n",
        ],
    );
    assert_reported(
        DEFAULT,
        "a.mts",
        &[
            "class A { field = [function () { return this }] }\n",
            "class A { static { const inner = function () { return this } } }\n",
        ],
    );
}

#[test]
fn the_key_and_the_decorators_of_a_class_member_have_the_this_of_the_code_around_the_class() {
    assert_reported(
        DEFAULT,
        "a.mts",
        &[
            "class A { [this.key] = 1 }\n",
            "class A { static [this.key]() {} }\n",
            "class A { get [this.key]() { return 1 } }\n",
            "class A { @decorate(this) method() {} }\n",
            "class A { @decorate(() => this) field = 1 }\n",
            "class A { accessor [this.key] = 1 }\n",
            "const object = { [this.key]: 1 }\n",
            "const object = { [this.key]() {} }\n",
            "@decorate(this) class A {}\n",
        ],
    );
    assert_valid(
        DEFAULT,
        "a.mts",
        &[
            "class A { method() { return class { [this.key] = 1 } } }\n",
            "class A { method(@decorate(this) parameter: number) {} }\n",
        ],
    );
}

#[test]
fn this_in_the_value_of_an_accessor_property_is_valid() {
    assert_valid(
        DEFAULT,
        "a.mts",
        &[
            "class A { accessor value = this.a; static accessor shared = this }\n",
            "class A { accessor value = function () { return () => this } }\n",
            "class A { accessor value = (function () { return this }) }\n",
            "class A { accessor value = () => this }\n",
            "class A { accessor value = { method: function () { return this } } }\n",
        ],
    );
}

#[test]
fn a_function_that_the_value_of_an_accessor_property_evaluates_to_is_reported() {
    assert_reported(
        DEFAULT,
        "a.mts",
        &[
            "class A { accessor value = cond ? function () { return this } : null }\n",
            "class A { accessor value = function () { return this } as Handler }\n",
            "class A { accessor value = (function () { return this })! }\n",
            "class A { accessor value = () => function () { return this } }\n",
        ],
    );
}

#[test]
fn a_function_without_a_body_starts_no_scope() {
    assert_reported(DEFAULT, "a.mts", &["declare function declared(a?: typeof this): void\n"]);
    assert_valid(
        DEFAULT,
        "a.mts",
        &["abstract class A { abstract method(a: number): void; other(a = this) { return a } }\n"],
    );
}

// --- What gives a function its `this`

#[test]
fn a_capitalized_function_is_a_constructor() {
    let cases = [
        "function Constructor() { this.a = 0 }\n",
        "const Constructor = function () { this.a = 0 }\n",
        "const lower = function Constructor() { this.a = 0 }\n",
        "let Constructor\nConstructor = function () { return this }\n",
        "const { Constructor = function () { return this } } = object\n",
        "function defaults(Callback = function () { return this }) {}\n",
        "const Constructor = (function () { return function () { return this } })()\n",
        ";[Constructor = function () { return this }] = []\n",
        ";({ Constructor = function () { return this } } = object)\n",
        "\u{c9}t\u{e9} = function () { return this }\n",
    ];
    assert_valid(DEFAULT, "a.mjs", &cases);
    assert_valid(r#"[{"capIsConstructor":true}]"#, "a.mjs", &cases);
    assert_valid("[{}]", "a.mjs", &cases);
    assert_reported(NO_CONSTRUCTORS, "a.mjs", &cases);
}

#[test]
fn a_function_with_a_name_is_not_the_constructor_that_its_variable_names() {
    assert_reported(
        DEFAULT,
        "a.mjs",
        &[
            "const Constructor = function lower() { this.a = 0 }\n",
            "const _Private = function () { this.a = 0 }\n",
            "const $Dollar = function () { this.a = 0 }\n",
        ],
    );
}

#[test]
fn without_cap_is_constructor_what_else_gives_a_this_still_does() {
    assert_valid(
        NO_CONSTRUCTORS,
        "a.mjs",
        &[
            "Constructor.prototype.method = function () { return this }\n",
            "const Bound = function () { return this }.bind(object)\n",
            "class A { Method() { return this } }\n",
        ],
    );
}

#[test]
fn a_this_parameter_types_the_this_of_a_function() {
    assert_valid(
        DEFAULT,
        "a.mts",
        &[
            "function typed(this: Context) { return this.value }\n",
            "const typed = function (this: Context, extra: number) { return this.value + extra }\n",
            "function typed(this: Context) { return () => this }\n",
            "function typed<T>(this: T): void\nfunction typed<T>(this: T, extra?: number) { use(this, extra) }\n",
        ],
    );
    assert_reported(
        DEFAULT,
        "a.mts",
        &[
            "function typed(this: Context) { return function () { return this } }\n",
            "function typed(this: Context): void\nfunction typed(extra?: number) { use(this, extra) }\n",
        ],
    );
}

#[test]
fn a_function_assigned_to_a_member_is_a_method() {
    assert_valid(
        DEFAULT,
        "a.mts",
        &[
            "object.method = function () { return this }\n",
            "object.nested['computed'] = function () { return this }\n",
            "Constructor.prototype.method = function () { return this }\n",
            "object.method ??= function () { return this }\n",
            "object.method = cond ? function () { return this } : null\n",
            "object.method = cond || function () { return this }\n",
            ";[object.method = function () { return this }] = []\n",
            ";({ key: object.method = function () { return this } } = source)\n",
            ";(object as Context).method = function () { return this }\n",
            "object!.method = function () { return this } satisfies Handler\n",
            "const object = { method: cond ? function () { return this } : function () { return this } }\n",
        ],
    );
}

#[test]
fn a_function_assigned_to_a_lower_case_name_or_to_a_pattern_is_reported() {
    assert_reported(
        DEFAULT,
        "a.mts",
        &[
            "let lower\nlower = function () { return this }\n",
            "lower += function () { return this }\n",
            ";(lower!) = function () { return this }\n",
            "const { lower = function () { return this } } = object\n",
            "const [{ a } = function () { return this }] = list\n",
            "const lower: Handler = function () { return this }\n",
            "object.method = (0, function () { return this })\n",
        ],
    );
}

#[test]
fn bind_call_and_apply_with_a_this_argument_give_a_this() {
    assert_valid(
        DEFAULT,
        "a.mts",
        &[
            "const bound = function () { return this }.bind(object)\n",
            "const called = (function () { return this }).call(object)\n",
            "const applied = (function () { return this }).apply(object, [])\n",
            "const optional = (function () { return this })?.call(object)\n",
            "const optional = (function () { return this }).call?.(object)\n",
            "const computed = (function () { return this })['call'](object)\n",
            "const computed = (function () { return this })[`apply`](object)\n",
            "const spread = (function () { return this }).call(...args)\n",
            "const grouped = ((function () { return this })?.call)(object)\n",
            "const typed = (function () { return this } as Handler).call(context as Context)\n",
        ],
    );
}

#[test]
fn bind_call_and_apply_without_a_this_are_reported() {
    assert_reported(
        DEFAULT,
        "a.mts",
        &[
            "const bound = function () { return this }.bind(null)\n",
            "const bound = function () { return this }.bind(undefined)\n",
            "const bound = function () { return this }.bind(void 0)\n",
            "const bound = function () { return this }.bind()\n",
            "const typed = (function () { return this }).call(null as never)\n",
            "const typed = (function () { return this }).call((undefined)!)\n",
            "const read = (function () { return this }).bind\n",
            "const other = (function () { return this }).toString(object)\n",
            "const dynamic = (function () { return this })[call](object)\n",
            "const asserted = (function () { return this }).call!(object)\n",
            "const chained = (function () { return this }).call.call(object)\n",
        ],
    );
}

#[test]
fn reflect_apply_with_a_this_argument_gives_a_this() {
    assert_valid(
        DEFAULT,
        "a.mjs",
        &[
            "Reflect.apply(function () { return this }, object, [])\n",
            "Reflect['apply']((function () { return this }), object, [])\n",
        ],
    );
    assert_reported(
        DEFAULT,
        "a.mjs",
        &[
            "Reflect.apply(function () { return this }, null, [])\n",
            "Reflect.apply(function () { return this }, object)\n",
            "Reflect.apply(other, function () { return this }, [])\n",
            "Reflect.apply(list.map, function () { return this }, object)\n",
        ],
    );
}

#[test]
fn array_from_with_a_this_argument_gives_a_this() {
    assert_valid(
        DEFAULT,
        "a.mjs",
        &[
            "Array.from([], function () { return this }, object)\n",
            "Int8Array.from([], function () { return this }, object)\n",
            "Array.fromAsync([], function () { return this }, object)\n",
            "(Array).from([], function () { return this }, object)\n",
        ],
    );
    assert_reported(
        DEFAULT,
        "a.mjs",
        &[
            "Array.from([], function () { return this })\n",
            "Array.from(function () { return this }, null, object)\n",
            "Int8Array.fromAsync([], function () { return this }, object)\n",
            "Arrays.from([], function () { return this }, object)\n",
            "globalThis.Array.from([], function () { return this }, object)\n",
        ],
    );
}

#[test]
fn an_array_method_with_a_this_argument_gives_a_this() {
    let methods = [
        "every",
        "filter",
        "find",
        "findIndex",
        "findLast",
        "findLastIndex",
        "flatMap",
        "forEach",
        "map",
        "some",
    ];
    let calls =
        methods.map(|method| format!("list.{method}(function () {{ return this }}, object)\n"));
    assert_valid(DEFAULT, "a.mts", &calls.each_ref().map(String::as_str));
    assert_valid(
        DEFAULT,
        "a.mts",
        &[
            "list?.map(function () { return this }, object)\n",
            "list.map?.(function () { return this }, object)\n",
            "list['map'](function () { return this }, object)\n",
            "list.map(function () { return this }, ...rest)\n",
            "list.map<number>(function () { return this } as Handler, object)\n",
            ";(list.map)(function () { return this }, object)\n",
        ],
    );
}

#[test]
fn an_array_method_without_a_this_argument_is_reported() {
    assert_reported(
        DEFAULT,
        "a.mts",
        &[
            "list.forEach(function () { return this })\n",
            "list.forEach(function () { return this }, null)\n",
            "list.map(function () { return this }, null!)\n",
            "list.map(function () { return this }, object, extra)\n",
            "list.map(other, function () { return this })\n",
            "list.reduce(function () { return this }, object)\n",
            "list.sort(function () { return this }, object)\n",
            "list.map!(function () { return this }, object)\n",
        ],
    );
}

#[test]
fn a_function_returned_by_a_function_called_in_place_is_what_the_call_is() {
    assert_valid(
        DEFAULT,
        "a.mts",
        &[
            "object.method = (function () { return function () { return this } })()\n",
            "object.method = (() => function () { return this })()\n",
            "object.method = (() => cond || function () { return this })()\n",
            "object.method = (() => { return function () { return this } })()\n",
            "object.method = ((() => function () { return this }) as () => Handler)()\n",
            "object.method = (function () { return function () { return this } })!()\n",
            "const object = { method: (function () { return function () { return this } })() }\n",
        ],
    );
    assert_reported(
        DEFAULT,
        "a.mts",
        &[
            "const lower = (function () { return function () { return this } })()\n",
            "object.method = () => function () { return this }\n",
            "function declaration() { return function () { return this } }\n",
            "const object = { method() { return function () { return this } } }\n",
        ],
    );
}

#[test]
fn a_function_under_a_type_assertion_is_what_the_assertion_is() {
    assert_reported(
        DEFAULT,
        "a.mts",
        &[
            "const asserted = function () { return this } as Handler\n",
            "const asserted = function () { return this }!\n",
            "const instantiated = (function <T>() { return this })<string>\n",
            "class A extends (function () { return this })<string> {}\n",
        ],
    );
    assert_valid(
        DEFAULT,
        "a.mts",
        &[
            "const Asserted = <Handler>function () { return this }\n",
            "const Instantiated = (function <T>() { return this })<string>\n",
        ],
    );
}

// --- `@this` comments

#[test]
fn a_comment_with_a_this_tag_before_a_function_types_its_this() {
    assert_valid(
        DEFAULT,
        "a.mjs",
        &[
            "/** @this Klass */\nfunction documented() { return this }\n",
            "/* @this Klass */\nfunction documented() { return this }\n",
            "// @this Klass\nfunction documented() { return this }\n",
            "/** @this Klass */\n\n\nfunction documented() { return this }\n",
            "/**\n * Description.\n *   @this {Klass}\n */\nfunction documented() { return this }\n",
            "use(/* @this Klass */ function () { return this })\n",
            "/** @this Klass */\nexport function documented() { return this }\n",
            "export /** @this Klass */ async function documented() { return this }\n",
            "/** @this Klass */\nexport default function () { return this }\n",
        ],
    );
}

#[test]
fn a_comment_without_a_this_tag_at_the_start_of_a_line_types_nothing() {
    assert_reported(
        DEFAULT,
        "a.mjs",
        &[
            "/** @returns {Klass} this */\nfunction documented() { return this }\n",
            "/** The @this of the function. */\nfunction documented() { return this }\n",
            "/** @this Klass */\nuse(function () { return this })\n",
            "const value = /** @this Klass */ (function () { return this })\n",
        ],
    );
}

#[test]
fn a_tag_that_starts_with_this_is_a_this_tag() {
    assert_valid(
        DEFAULT,
        "a.mjs",
        &["/** @thisIsNotATag */\nfunction documented() { return this }\n"],
    );
}

#[test]
fn the_jsdoc_of_the_statement_or_of_the_property_of_a_function_expression_types_its_this() {
    assert_valid(
        DEFAULT,
        "a.mjs",
        &[
            "/** @this Klass */\nconst documented = function () { return this }\n",
            "/** @this Klass */\nexport const documented = cond || function () { return this }\n",
            "let first, /** @this Klass */ second = function () { return this }\n",
            "const object = {\n  /** @this Klass */\n  list: [function () { return this }],\n}\n",
            "const { /** @this Klass */ documented = function () { return this } } = object\n",
            "const { key: /** @this Klass */ documented = function () { return this } } = object\n",
            "function defaults(/** @this Klass */ callback = function () { return this }) {}\n",
            "function outer() {\n  /** @this Klass */\n  return cond || function () { return this }\n}\n",
            "/** @this Klass */\nuse(cond ? function () { return this } : null)\n",
        ],
    );
}

#[test]
fn a_jsdoc_that_is_not_right_above_the_statement_of_a_function_expression_types_nothing() {
    assert_reported(
        DEFAULT,
        "a.mjs",
        &[
            "/** @this Klass */\n\nconst documented = function () { return this }\n",
            "/** @this Klass */ // other\nconst documented = function () { return this }\n",
            "/* @this Klass */\nconst documented = function () { return this }\n",
            "// @this Klass\nconst documented = function () { return this }\n",
            "/** @this Klass */\nconst holder = () => function () { return this }\n",
            "/** @this Klass */\nconst outer = function () { return function () { return this } }\n",
            "class A {\n  /** @this Klass */\n  method() { return cond || function () { return this } }\n}\n",
        ],
    );
}

#[test]
fn the_line_of_a_jsdoc_is_counted_with_carriage_returns() {
    assert_valid(
        DEFAULT,
        "a.mjs",
        &["/** @this Klass */\r\nconst documented = function () { return this }\r\n"],
    );
    assert_reported(
        DEFAULT,
        "a.mjs",
        &["/** @this Klass */\r\n\r\nconst documented = function () { return this }\r\n"],
    );
}

#[test]
fn a_type_that_the_parser_takes_from_a_jsdoc_in_a_js_file_is_not_code() {
    assert_valid(
        DEFAULT,
        "a.mjs",
        &[
            "/** @type {typeof this} */\nconst typed = 1\n",
            "object.method = /** @type {Handler} */ (function () { return this })\n",
            ";(/** @type {any} */ (list.map))(function () { return this }, object)\n",
        ],
    );
    assert_reported(
        DEFAULT,
        "a.mjs",
        &["const lower = /** @type {Handler} */ (function () { return this })\n"],
    );
}

// --- What is a `this` expression

#[test]
fn this_in_a_type_query_is_a_this_expression() {
    assert_reported(
        DEFAULT,
        "a.mts",
        &[
            "type Top = typeof this\n",
            "let value: typeof this.a.b\n",
            "function lower() { let value: typeof this; return value }\n",
            "class A { field: typeof this = 1 }\n",
        ],
    );
    assert_valid(
        DEFAULT,
        "a.mts",
        &[
            "class A { method(a: typeof this) { return a } }\n",
            "function typed(this: Context) { let value: typeof this.value; return value }\n",
        ],
    );
}

#[test]
fn the_this_type_and_the_this_parameter_are_not_this_expressions() {
    assert_valid(
        DEFAULT,
        "a.mts",
        &[
            "interface Context { method(this: Context): this }\n",
            "type Handler = (this: Context) => void\n",
            "declare function guard(this: Context): this is Context\n",
        ],
    );
}

#[test]
fn this_in_the_name_of_a_jsx_element_is_not_a_this_expression() {
    assert_valid(
        DEFAULT,
        "a.tsx",
        &[
            "export const element = <this.Component.Part></this.Component.Part>\n",
            "export const element = <this />\n",
        ],
    );
    assert_reported(
        DEFAULT,
        "a.tsx",
        &[
            "export const element = <div ref={this} />\n",
            "export const element = <div onClick={function () { return this }} />\n",
        ],
    );
}

// --- Options

#[test]
fn options_that_the_rule_does_not_know_are_refused() {
    assert_eq!(options_error("[]"), None);
    assert_eq!(options_error("[{}]"), None);
    assert_eq!(options_error(r#"[{"capIsConstructor":false}]"#), None);
    assert_eq!(
        options_error(r#"[{"capIsConstructor":"no"}]"#).as_deref(),
        Some("js/no-invalid-this: the option `capIsConstructor` is a boolean")
    );
    assert_eq!(
        options_error(r#"[{"capIsConstructors":false}]"#).as_deref(),
        Some("js/no-invalid-this: unknown option `capIsConstructors`")
    );
    assert_eq!(
        options_error("[false]").as_deref(),
        Some("js/no-invalid-this: the options are an object, `{ capIsConstructor }`")
    );
    assert_eq!(
        options_error("[{}, {}]").as_deref(),
        Some("js/no-invalid-this: takes one options object, `{ capIsConstructor }`")
    );
}
