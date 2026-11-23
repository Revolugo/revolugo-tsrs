//! `js/import-export`: what the rule reports, case by case. The cases are those of the JS rule's fixtures
//! (`revolugo-lint-plugins`, `test/import/fixtures/workspace`) and of each branch of its source.

use tsrs_core::json;
use tsrs_lint::payload::ConfiguredRule;

use super::{Reported, lint, project};

/// What the rule reports in `file` of a project made of `files`: the text it marks and its message.
fn reported(test: &str, files: &[(&str, &str)], file: &str) -> Reported {
    let base = project(&format!("import-export-{test}"), files);
    lint(&base, file, "js/import-export", "[]")
}

/// What the rule reports in `source`, alone in a project.
fn reported_in(test: &str, source: &str) -> Reported {
    reported(test, &[("a.ts", source)], "a.ts")
}

/// The reports of the exports of `name`, one for each text in `marked`.
fn duplicates(name: &str, marked: &[&str]) -> Reported {
    let message = if name == "default" {
        "Multiple default exports.".to_string()
    } else {
        format!("Multiple exports of name '{name}'.")
    };
    marked.iter().map(|text| ((*text).to_string(), message.clone())).collect()
}

/// The error that the rule gives for `options`.
fn refusal(options: &str) -> Option<String> {
    let options = json::unmarshal(options).unwrap();
    let config = ConfiguredRule { name: "js/import-export".into(), options };
    match tsrs_lint::ported::create_rule(&config) {
        Some(Err(error)) => Some(error),
        Some(Ok(_)) => None,
        None => panic!("js/import-export is not a ported rule"),
    }
}

const FIRST: (&str, &str) =
    ("first.ts", "export const shared = 'first'\nexport const onlyFirst = 1\n");
const SECOND: (&str, &str) = (
    "second.ts",
    "export const shared = 'second'\nexport const onlySecond = 2\n\nexport default 'second'\n",
);

// --- The exports of the file itself.

#[test]
fn a_name_exported_twice_is_reported_on_each_export() {
    let source = "const first = 1\nconst second = 2\nexport { first as shared }\nexport { second as shared }\n";
    assert_eq!(reported_in("twice", source), duplicates("shared", &["shared", "shared"]));
}

#[test]
fn a_name_may_be_a_string() {
    let source = "const a = 1\nexport { a as 'x y' }\nexport { a as 'x y' }\n";
    assert_eq!(reported_in("string-name", source), duplicates("x y", &["'x y'", "'x y'"]));
}

#[test]
fn a_second_default_export_is_reported() {
    let source = "export const value = 1\n\nexport default value\nexport { value as default }\n";
    assert_eq!(
        reported_in("default", source),
        duplicates("default", &["export default value", "default"])
    );
}

#[test]
fn a_default_declaration_is_marked_from_export_to_its_end() {
    let source = "const a = 1\ndeclare const dec: any\n@dec\nexport default class A {}\nexport { a as default }\n";
    assert_eq!(
        reported_in("default-class", source),
        duplicates("default", &["export default class A {}", "default"])
    );
}

#[test]
fn the_overloads_of_a_function_are_one_export() {
    let source = "export function f(a: string): string\nexport function f(a: number): number\nexport function f(a: unknown): unknown {\n  return a\n}\n\nexport declare function g(a: string): void\nexport declare function g(a: number): void\n\nexport default function h(a: string): void\nexport default function h(a: unknown): void {}\n";
    assert_eq!(reported_in("overloads", source), []);
}

#[test]
fn overloads_without_implementation_count_once_against_another_export() {
    let source = "export declare function f(a: string): void\nexport declare function f(a: number): void\nconst g = 1\nexport { g as f }\n";
    assert_eq!(reported_in("overloads-only", source), duplicates("f", &["f", "f"]));
}

#[test]
fn a_namespace_merges_with_a_function_a_class_or_an_enum() {
    let source = "export namespace Merged {\n  export const first = 1\n}\nexport namespace Merged {\n  export const second = 2\n}\n\nexport class Model {}\nexport namespace Model {\n  export const version = 1\n}\n\nexport enum Level {\n  Low,\n}\nexport namespace Level {\n  export const fallback = Level.Low\n}\n\nexport function helper(a: string): void\nexport function helper(a: unknown): void {}\nexport namespace helper {\n  export const version = 1\n}\n";
    assert_eq!(reported_in("merging", source), []);
}

#[test]
fn a_namespace_next_to_two_declarations_it_merges_with_is_not_reported() {
    let source = "export enum Flags {\n  A,\n}\nexport enum Flags {\n  B = 1,\n}\nexport namespace Flags {\n  export const all = [Flags.A, Flags.B]\n}\n";
    assert_eq!(reported_in("two-enums", source), duplicates("Flags", &["Flags", "Flags"]));
}

#[test]
fn a_namespace_and_a_value_it_does_not_merge_with_are_duplicates() {
    let source = "const a = 1\nexport namespace N {\n  export const v = 1\n}\nexport { a as N }\n";
    assert_eq!(reported_in("namespace-value", source), duplicates("N", &["N", "N"]));
}

#[test]
fn a_namespace_that_declares_only_types_exports_no_value() {
    let source = "namespace M {\n  export type T = 1\n}\nexport namespace Types {\n  import T = M.T\n  export interface Entry {}\n  export namespace Inner {\n    type U = T\n  }\n}\nexport namespace Empty {}\nconst v = 1\nexport { v as Types, v as Empty }\n";
    assert_eq!(reported_in("type-namespace", source), []);
}

#[test]
fn a_namespace_with_a_dotted_name_is_never_a_duplicate() {
    let source = "export namespace A.B {\n  export const x = 1\n}\nexport namespace A.B {\n  export const y = 1\n}\nconst v = 1\nexport { v as A }\n";
    assert_eq!(reported_in("dotted", source), []);
}

#[test]
fn a_type_and_a_value_share_a_name() {
    let source = "export type Shape = string\nexport const Shape = 'shape'\n\nexport class Model {}\nexport interface Model {\n  a: string\n}\n";
    assert_eq!(reported_in("type-and-value", source), []);
}

#[test]
fn an_interface_declared_twice_is_reported() {
    let source =
        "export interface Merged {\n  a: string\n}\nexport interface Merged {\n  b: string\n}\n";
    assert_eq!(reported_in("interface", source), duplicates("Merged", &["Merged", "Merged"]));
}

#[test]
fn export_type_next_to_a_value_exports_the_same_binding() {
    let source = "const Both = 1\ntype Both = string\nexport type { Both }\nexport { Both }\n";
    // The value first, then the type that joins it.
    assert_eq!(reported_in("type-specifier", source), duplicates("Both", &["Both", "Both"]));
}

#[test]
fn export_type_next_to_overloads_only_is_not_a_duplicate() {
    let source = "export declare function f(a: string): void\ntype T = 1\nexport type { T as f }\n";
    assert_eq!(reported_in("type-overload", source), []);
}

#[test]
fn each_namespace_has_its_own_exports() {
    let source = "export namespace Api {\n  export interface Entry {\n    a: string\n  }\n  export interface Entry {\n    b: string\n  }\n  export const once = 1\n}\nexport const once = 2\nexport interface Entry {\n  c: string\n}\n";
    assert_eq!(reported_in("scopes", source), duplicates("Entry", &["Entry", "Entry"]));
}

#[test]
fn the_names_of_a_destructuring_pattern_are_exports() {
    let source = "declare const source: any\nexport const { a, b: { c }, d = 1, ...e } = source\nexport const [f, , g = 2, ...h] = source\nconst q = 1\nexport { q as c, q as d, q as e, q as g, q as h }\n";
    assert_eq!(
        reported_in("patterns", source),
        [
            duplicates("c", &["c", "c"]),
            duplicates("d", &["d", "d"]),
            duplicates("e", &["e", "e"]),
            duplicates("g", &["g", "g"]),
            duplicates("h", &["h", "h"]),
        ]
        .concat()
    );
}

#[test]
fn an_annotated_variable_is_marked_with_its_type() {
    let source =
        "export let a: number\nexport let b!: string\nconst q = 1\nexport { q as a, q as b }\n";
    assert_eq!(
        reported_in("annotations", source),
        [duplicates("a", &["a: number", "a"]), duplicates("b", &["b!: string", "b"])].concat()
    );
}

#[test]
fn an_exported_import_alias_is_a_value() {
    let source = "namespace M {\n  export const x = 1\n}\nexport import X = M.x\nconst q = 1\nexport { q as X }\n";
    assert_eq!(reported_in("import-alias", source), duplicates("X", &["X", "X"]));
}

#[test]
fn an_export_in_a_block_is_an_export_of_the_module() {
    let source = "const q = 1\n{\n  export { q as a }\n}\nexport { q as a }\n";
    assert_eq!(reported_in("block", source), duplicates("a", &["a", "a"]));
}

#[test]
fn export_equals_and_export_star_as_export_no_name_to_compare() {
    let files =
        [FIRST, ("a.ts", "export * as shared from './first.ts'\nexport const shared = 1\n")];
    assert_eq!(reported("star-as", &files, "a.ts"), []);
    assert_eq!(reported_in("export-equals", "const a = 1\nexport = a\n"), []);
}

#[test]
fn export_star_as_a_string_is_read_as_export_star() {
    let files = [FIRST, ("a.ts", "export * as 'ns' from './first.ts'\nexport const shared = 1\n")];
    assert_eq!(
        reported("star-as-string", &files, "a.ts"),
        duplicates("shared", &["shared", "export * as 'ns' from './first.ts'"])
    );
}

// --- The names that `export *` brings in.

#[test]
fn a_name_of_an_export_star_and_of_the_file_is_reported() {
    let files = [FIRST, ("a.ts", "export * from './first.ts'\nexport const shared = 'local'\n")];
    assert_eq!(
        reported("star-and-local", &files, "a.ts"),
        duplicates("shared", &["shared", "export * from './first.ts'"])
    );
}

#[test]
fn a_name_that_two_export_stars_bring_in_is_reported() {
    let files =
        [FIRST, SECOND, ("a.ts", "export * from './first.ts'\nexport * from './second.ts'\n")];
    assert_eq!(
        reported("two-stars", &files, "a.ts"),
        duplicates("shared", &["export * from './first.ts'", "export * from './second.ts'"])
    );
}

#[test]
fn a_name_comes_through_the_export_star_of_the_module() {
    let files = [
        FIRST,
        ("barrel.ts", "export * from './first.ts'\n"),
        ("a.ts", "export * from './barrel.ts'\nexport const onlyFirst = 'local'\n"),
    ];
    assert_eq!(
        reported("nested", &files, "a.ts"),
        duplicates("onlyFirst", &["onlyFirst", "export * from './barrel.ts'"])
    );
}

#[test]
fn modules_that_re_export_each_other_are_read_once() {
    let files = [
        ("a.ts", "export * from './b.ts'\nexport const fromA = 'a'\n"),
        ("b.ts", "export * from './a.ts'\nexport const fromB = 'b'\n"),
    ];
    assert_eq!(
        reported("cycle", &files, "a.ts"),
        duplicates("fromA", &["fromA", "export * from './b.ts'"])
    );
}

#[test]
fn an_export_star_in_an_ambient_module_belongs_to_that_module() {
    let files = [
        FIRST,
        SECOND,
        (
            "a.ts",
            "declare module 'm' {\n  export * from './first.ts'\n  export * from './second.ts'\n}\nexport * from './first.ts'\n",
        ),
    ];
    assert_eq!(
        reported("ambient-module", &files, "a.ts"),
        duplicates("shared", &["export * from './first.ts'", "export * from './second.ts'"])
    );
}

#[test]
fn a_module_without_a_named_export_is_reported() {
    let files = [
        ("default-only.ts", "export default 'default only'\n"),
        ("empty.ts", "export {}\n"),
        ("a.ts", "export * from './default-only.ts'\nexport * from './empty.ts'\n"),
    ];
    assert_eq!(
        reported("no-named", &files, "a.ts"),
        [
            (
                "'./default-only.ts'".to_string(),
                "No named exports found in module './default-only.ts'.".to_string()
            ),
            (
                "'./empty.ts'".to_string(),
                "No named exports found in module './empty.ts'.".to_string()
            ),
        ]
    );
}

#[test]
fn a_module_whose_names_are_not_all_known_is_not_called_empty() {
    let files = [
        ("unknown.ts", "export * from 'a-package-that-does-not-resolve'\n"),
        ("a.ts", "export * from './unknown.ts'\n"),
    ];
    assert_eq!(reported("unknown-names", &files, "a.ts"), []);
}

#[test]
fn export_star_does_not_bring_in_a_default_export() {
    let files = [SECOND, ("a.ts", "export * from './second.ts'\nexport default 'local'\n")];
    assert_eq!(reported("star-default", &files, "a.ts"), []);
}

#[test]
fn a_type_of_an_export_star_and_a_value_of_the_file_share_a_name() {
    let files = [
        ("types.ts", "export type Shape = 'round'\nexport interface Size {\n  width: number\n}\n"),
        (
            "a.ts",
            "export * from './types.ts'\nexport const Shape = 'round'\nexport type Size = number\n",
        ),
    ];
    assert_eq!(
        reported("star-type", &files, "a.ts"),
        duplicates("Size", &["Size", "export * from './types.ts'"])
    );
}

#[test]
fn export_type_star_brings_in_no_value() {
    let files = [
        FIRST,
        (
            "a.ts",
            "export type * from './first.ts'\nexport const shared = 1\nexport type onlyFirst = 1\n",
        ),
    ];
    assert_eq!(reported("type-star", &files, "a.ts"), []);
}

#[test]
fn a_class_of_an_export_star_is_a_value_and_a_type() {
    let files = [
        ("model.ts", "export class Model {}\nexport enum Level {\n  Low,\n}\n"),
        (
            "a.ts",
            "export * from './model.ts'\nexport type Model = 1\nconst level = 1\nexport { level as Level }\n",
        ),
    ];
    assert_eq!(
        reported("star-class", &files, "a.ts"),
        [
            duplicates("Level", &["Level", "export * from './model.ts'"]),
            duplicates("Model", &["Model", "export * from './model.ts'"]),
        ]
        .concat()
    );
}

#[test]
fn a_class_of_the_file_is_a_type_that_an_export_star_may_clash_with() {
    let files = [
        ("types.ts", "export interface Model {\n  a: 1\n}\n"),
        ("a.ts", "export * from './types.ts'\nexport class Model {}\n"),
    ];
    assert_eq!(
        reported("class-and-star-type", &files, "a.ts"),
        duplicates("Model", &["export * from './types.ts'", "Model"])
    );
}

#[test]
fn a_re_exported_name_is_what_it_is_in_the_module_it_comes_from() {
    let files = [
        ("types.ts", "export interface T {\n  a: 1\n}\nexport const v = 1\n"),
        (
            "barrel.ts",
            "import { T, v } from './types.ts'\nimport type { v as typeOfV } from './types.ts'\nexport { T, v, typeOfV }\nexport { T as U } from './types.ts'\n",
        ),
        (
            "a.ts",
            "export * from './barrel.ts'\nexport const T = 1\nexport const U = 1\nexport const typeOfV = 1\nexport const v = 2\n",
        ),
    ];
    assert_eq!(
        reported("followed", &files, "a.ts"),
        duplicates("v", &["v", "export * from './barrel.ts'"])
    );
}

#[test]
fn a_name_from_a_module_that_cannot_be_read_counts_as_a_value() {
    let files = [
        (
            "barrel.ts",
            "export { gone } from './missing.ts'\nexport type { gone as goneType } from './missing.ts'\n",
        ),
        (
            "a.ts",
            "export * from './barrel.ts'\nexport const gone = 1\nexport type goneType = 1\nexport const goneValue = 1\n",
        ),
    ];
    assert_eq!(
        reported("unreadable", &files, "a.ts"),
        [
            duplicates("gone", &["gone", "export * from './barrel.ts'"]),
            duplicates("goneType", &["goneType", "export * from './barrel.ts'"]),
        ]
        .concat()
    );
}

// --- Which file an `export *` names.

#[test]
fn a_specifier_resolves_as_node_does_without_the_ts_extension() {
    let files = [
        FIRST,
        ("dir/index.ts", "export const shared = 1\n"),
        (
            "a.ts",
            "export * from './first'\nexport * from './first.js'\nexport * from './dir'\nexport const shared = 1\n",
        ),
    ];
    assert_eq!(reported("extensions", &files, "a.ts"), []);
}

#[test]
fn a_directory_resolves_to_its_main_file_or_its_index() {
    let files = [
        ("dir/index.js", "export const fromIndex = 1\n"),
        ("pkg/package.json", r#"{ "main": "lib/entry" }"#),
        ("pkg/lib/entry.js", "export const fromMain = 1\n"),
        (
            "a.ts",
            "export * from './dir'\nexport * from './pkg/'\nexport const fromIndex = 2\nexport const fromMain = 2\n",
        ),
    ];
    assert_eq!(
        reported("directories", &files, "a.ts"),
        [
            duplicates("fromIndex", &["fromIndex", "export * from './dir'"]),
            duplicates("fromMain", &["fromMain", "export * from './pkg/'"]),
        ]
        .concat()
    );
}

#[test]
fn a_package_is_read_from_the_file_its_exports_give_to_import() {
    let files = [
        (
            "node_modules/pkg/package.json",
            r#"{ "exports": { ".": { "types": "./types.d.ts", "import": "./esm.js", "default": "./cjs.js" }, "./features/*": "./src/*.js" } }"#,
        ),
        ("node_modules/pkg/types.d.ts", "export declare const onlyInTypes: number\n"),
        ("node_modules/pkg/esm.js", "export const fromPackage = 1\n"),
        ("node_modules/pkg/cjs.js", "module.exports = {}\n"),
        ("node_modules/pkg/src/a.js", "export const featureA = 1\n"),
        (
            "a.ts",
            "export * from 'pkg'\nexport * from 'pkg/features/a'\nexport const fromPackage = 2\nexport const featureA = 2\nexport const onlyInTypes = 2\n",
        ),
    ];
    assert_eq!(
        reported("package-exports", &files, "a.ts"),
        [
            duplicates("featureA", &["featureA", "export * from 'pkg/features/a'"]),
            duplicates("fromPackage", &["fromPackage", "export * from 'pkg'"]),
        ]
        .concat()
    );
}

#[test]
fn a_package_without_exports_is_read_from_its_module_or_main_field() {
    let files = [
        ("node_modules/@scope/pkg/package.json", r#"{ "main": "./cjs.js", "module": "./esm.js" }"#),
        ("node_modules/@scope/pkg/esm.js", "export const fromPackage = 1\n"),
        ("node_modules/@scope/pkg/cjs.js", "module.exports = {}\n"),
        ("node_modules/@scope/pkg/lib/file.js", "export const fromFile = 1\n"),
        (
            "src/a.ts",
            "export * from '@scope/pkg'\nexport * from '@scope/pkg/lib/file'\nexport const fromPackage = 2\nexport const fromFile = 2\n",
        ),
    ];
    assert_eq!(
        reported("package-main", &files, "src/a.ts"),
        [
            duplicates("fromFile", &["fromFile", "export * from '@scope/pkg/lib/file'"]),
            duplicates("fromPackage", &["fromPackage", "export * from '@scope/pkg'"]),
        ]
        .concat()
    );
}

#[test]
fn a_commonjs_package_an_unresolved_one_and_a_builtin_are_not_read() {
    let files = [
        ("node_modules/commonjs/index.js", "module.exports = { shared: 1 }\n"),
        ("node_modules/broken/package.json", "{ nope"),
        ("node_modules/broken/index.js", "export const shared = 1\n"),
        ("node_modules/fs/index.js", "export const shared = 1\n"),
        ("data.json", r#"{ "shared": 1 }"#),
        (
            "a.ts",
            "export * from 'commonjs'\nexport * from 'broken'\nexport * from 'fs'\nexport * from 'node:path'\nexport * from 'a-package-that-does-not-resolve'\nexport * from './data.json'\nexport const shared = 1\n",
        ),
    ];
    assert_eq!(reported("not-read", &files, "a.ts"), []);
}

#[test]
fn a_hash_specifier_resolves_through_the_imports_of_package_json() {
    let files = [
        ("package.json", r##"{ "imports": { "#first": "./first.ts", "#lib/*": "./*.ts" } }"##),
        FIRST,
        SECOND,
        (
            "a.ts",
            "export * from '#first'\nexport * from '#lib/second'\nexport * from '#components'\n",
        ),
    ];
    assert_eq!(
        reported("imports", &files, "a.ts"),
        duplicates("shared", &["export * from '#first'", "export * from '#lib/second'"])
    );
}

#[cfg(unix)]
#[test]
fn a_workspace_package_is_read_under_node_modules_through_its_link() {
    let files = [
        ("packages/lib/package.json", r#"{ "name": "lib", "exports": { ".": "./src/index.ts" } }"#),
        ("packages/lib/src/index.ts", "export * from './other.ts'\nexport * from 'dep'\n"),
        ("packages/lib/src/other.ts", "export const other = 1\n"),
        // A dependency that only the application has: the library finds it from the application's link.
        ("packages/app/node_modules/dep/index.js", "export const fromDep = 1\n"),
        (
            "packages/app/src/a.ts",
            "export * from 'lib'\nexport const other = 2\nexport const fromDep = 2\n",
        ),
    ];
    let base = project("import-export-workspace", &files);
    std::os::unix::fs::symlink(
        base.join("packages/lib"),
        base.join("packages/app/node_modules/lib"),
    )
    .unwrap();
    assert_eq!(
        lint(&base, "packages/app/src/a.ts", "js/import-export", "[]"),
        [
            duplicates("fromDep", &["fromDep", "export * from 'lib'"]),
            duplicates("other", &["other", "export * from 'lib'"]),
        ]
        .concat()
    );
}

// --- What is read of the module.

#[test]
fn a_javascript_module_is_read_without_what_its_jsdoc_declares() {
    let files = [
        (
            "source.js",
            "/** @typedef {string} Name */\n/**\n * @overload\n * @param {string} a\n * @returns {void}\n */\n/** @param {unknown} a */\nexport function starred(a) {}\n",
        ),
        (
            "a.ts",
            "export * from './source.js'\nexport const starred = 'local'\nexport type Name = 1\n",
        ),
    ];
    assert_eq!(
        reported("javascript", &files, "a.ts"),
        duplicates("starred", &["starred", "export * from './source.js'"])
    );
}

#[test]
fn the_script_blocks_of_a_vue_component_are_read() {
    let files = [
        FIRST,
        (
            "Widget.vue",
            "<template>\n  <p>{{ a < b }} é <template v-if=\"x\"><script>ignored</script></template></p>\n</template>\n\n<script lang=\"ts\">\nexport const shared = 'component'\n</script>\n\n<script setup lang=\"ts\">\nexport interface WidgetProps {\n  label: string\n}\n</script>\n",
        ),
        ("Plain.vue", "<template>\n  <p />\n</template>\n"),
        (
            "a.ts",
            "export * from './first.ts'\nexport * from './Widget.vue'\nexport * from './Plain.vue'\nexport const WidgetProps = 1\n",
        ),
    ];
    assert_eq!(
        reported("vue", &files, "a.ts"),
        [
            vec![(
                "'./Plain.vue'".to_string(),
                "No named exports found in module './Plain.vue'.".to_string()
            )],
            duplicates("shared", &["export * from './first.ts'", "export * from './Widget.vue'"]),
        ]
        .concat()
    );
}

#[test]
fn a_module_that_is_not_an_es_module_or_does_not_parse_is_not_read() {
    let files = [
        ("script.ts", "const shared = 1\n"),
        ("broken.ts", "export const = 1\n"),
        ("assigned.ts", "const a = { shared: 1 }\nexport = a\n"),
        ("Coffee.vue", "<script lang=\"coffee\">\nshared = 1\n</script>\n"),
        (
            "a.ts",
            "export * from './script.ts'\nexport * from './broken.ts'\nexport * from './assigned.ts'\nexport * from './Coffee.vue'\nexport const shared = 1\n",
        ),
    ];
    assert_eq!(reported("not-modules", &files, "a.ts"), []);
}

#[test]
fn every_way_to_export_a_name_is_read() {
    let files = [
        FIRST,
        (
            "forms.ts",
            "import def, * as ns from './first.ts'\nimport legacy = require('./first.ts')\nexport { def, ns, legacy }\nexport { shared as renamed } from './first.ts'\nexport * as star from './first.ts'\nexport const { a, b: [c, ...d] } = ns as any\nexport function fn(): void {}\nexport class Class {}\nexport enum Enum {\n  A,\n}\nexport interface Interface {\n  a: 1\n}\nexport type Alias = 1\nexport namespace Namespace {\n  export const x = 1\n}\nexport namespace Dotted.Name {\n  export const x = 1\n}\nnamespace Inner {\n  export const x = 1\n}\nexport import NotAnExport = Inner.x\nexport default fn\n",
        ),
        ("a.ts", "export * from './forms.ts'\nexport * from './forms.ts'\n"),
    ];
    let names: Vec<String> = reported("forms", &files, "a.ts")
        .into_iter()
        .step_by(2)
        .map(|(_, message)| message)
        .collect();
    let expected = [
        "Alias",
        "Class",
        "Enum",
        "Interface",
        "Namespace",
        "a",
        "c",
        "d",
        "def",
        "fn",
        "legacy",
        "ns",
        "renamed",
        "star",
    ];
    assert_eq!(names, expected.map(|name| format!("Multiple exports of name '{name}'.")));
}

// --- The options.

#[test]
fn the_rule_takes_no_option() {
    assert_eq!(refusal("[]"), None);
    assert_eq!(refusal("null"), None);
    assert_eq!(
        refusal(r#"[{ "anything": true }]"#).as_deref(),
        Some("js/import-export does not accept options: remove the option `anything`")
    );
    assert_eq!(
        refusal("[true]").as_deref(),
        Some("js/import-export does not accept options: remove its options")
    );
}
