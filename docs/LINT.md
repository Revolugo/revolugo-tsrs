# `tsrs-lint`: lint a project with every rule of its Oxlint configuration (revolugo fork)

`tsrs-lint` reads an Oxlint configuration file, runs every rule of it, and prints one report. It starts Oxlint itself
for Oxlint's native rules and for the rules of the JS plugins that it does not run itself. It runs three kinds of
rules without Oxlint:

- The rules of typescript-eslint that need the types (`typescript/no-floating-promises`), on the script files and
  on the `.vue` files. Oxlint sends no `.vue` file to its type-aware backend. `tsrs-lint` lints a `.vue` file through
  the content mapper of its tsconfig.
- The rules that a JS plugin leaves to `tsrs-lint` (`meta.native`), on the script files. `tsrs-lint` has a port of
  each of these rules in Rust. Oxlint runs the rules of a JS plugin on one thread, and `tsrs-lint` runs its rules on
  every core.
- The rules of the JS plugins that declare `meta.sfc: true`, on the `<template>` and the custom blocks of the `.vue`
  files. Oxlint parses only the `<script>` blocks of a `.vue` file.

The first two kinds run in the native linter, `tsrs-lint headless`, in one process for all the files. The third kind
runs in a rule host written in JavaScript. Oxlint, the native linter and the rule host run at the same time.
`--no-oxlint` leaves Oxlint out, for a client that runs Oxlint itself.

The command ships in the `tsrs` npm package. Its sources are in `npm/tsrs/src/tsrs/lint`.

## Usage

```sh
tsrs-lint [--fix] [--format unix|json] [--config <file>] [--root <dir>] [--project <tsconfig>] [--no-oxlint] [paths...]
tsrs-lint headless [--fix] [--runExternalCode] [--project <tsconfig>]
tsrs-lint rules
```

Run the command from the root of the project. The current directory is where the command lists the files, finds
`node_modules`, and runs Oxlint and the native linter.

`tsrs-lint headless` and `tsrs-lint rules` are the native linter itself. `headless` is the backend that Oxlint
drives with `--type-aware`: it reads a payload on stdin and writes frames on stdout
([tools/tsgolint/README.md](../tools/tsgolint/README.md)). `rules` lists the rules that the native linter
implements. A path named `headless` or `rules` is written `./headless` or `./rules`.

| option | meaning |
| --- | --- |
| `paths` | The files and directories to lint, relative to the current directory. The default is `.`. |
| `--fix` | Apply the fixes and write the files. The report is about the fixed files. The fixes of the native linter are applied to the script files only. |
| `--no-oxlint` | Do not run Oxlint: lint with the rules that Oxlint does not run, and nothing else. |
| `-f`, `--format` | `unix` (the default) or `json`. |
| `-c`, `--config` | The Oxlint configuration file. The default is `.oxlintrc.json` in the current directory. |
| `--root` | The directory that the configuration applies to. The default is the directory of the configuration file. It needs `--no-oxlint`. |
| `-p`, `--project` | The tsconfig that `tsrs-lint headless --project` gets. |
| `-h`, `--help` | Print the usage. |

## Requirements

- Node.js 22.18 or later. A plugin is an ES module, and Node.js 22.18 loads one that is written in TypeScript. The
  rest of the `tsrs` package, `tsrs-lint headless` and `tsrs-lint rules` included, still runs on Node.js 16.20.
- A git repository. The command lists the files with `git ls-files`.
- `oxlint` in the project, unless `--no-oxlint` is given. The command runs the `oxlint` command of the package that
  the project resolves, without `--type-aware`.
- `vue-eslint-parser` and `@typescript-eslint/parser` in the project, when a rule of an `sfc` plugin is on for a
  `.vue` file.
- `@vue/compiler-sfc` in the project, when the native linter reports something in a `.vue` file.

The `tsrs` package depends on none of these packages. The command resolves each one from the current directory, as a
module of the project does, and names the missing package when the project does not have it. None of them needs the
`eslint` package.

## Files

The command lints the files under `paths` that Oxlint lints too:

- The script files (`.js`, `.mjs`, `.cjs`, `.jsx`, `.ts`, `.mts`, `.cts`, `.tsx`) and the `.vue` files.
- Tracked files, and untracked files that `.gitignore` does not exclude (`git ls-files -co --exclude-standard`). A
  tracked file that `.gitignore` names is left out, as in Oxlint.
- Files that `ignorePatterns` does not match. An ignore pattern matches a file, or a directory at any depth: `dist`,
  `**/dist` and `apps/web/generated` all work.

## Configuration

The command reads these keys of the configuration file. It ignores every other key, `extends` included.

| key | use |
| --- | --- |
| `rules` | The rule entries of every file. |
| `overrides[].files`, `overrides[].excludeFiles`, `overrides[].rules` | The rule entries of the files that match. |
| `jsPlugins`, `overrides[].jsPlugins` | The plugins to load. |
| `settings` | Passed to the hosted rules as `context.settings`. |
| `ignorePatterns` | The files to leave out. |

The file must be JSON without comments.

The rule entries of a file are the base `rules`, then the `rules` of each override whose `files` match the file and
whose `excludeFiles` do not, in order. A later entry replaces an earlier one. A later entry that is a severity alone
keeps the options of the earlier one, unless the severity turns the rule off. A severity is `0`, `1`, `2`, `"off"`,
`"allow"`, `"warn"`, `"error"` or `"deny"`.

Globs and relative plugin paths are resolved against the directory of the configuration file, as in Oxlint. `--root`
names another directory. Use it for a configuration file that is not in the tree that it configures, a generated one
for instance. Oxlint has no such option, so `--root` needs `--no-oxlint`:

```sh
tsrs-lint --no-oxlint --config /tmp/generated.oxlintrc.json --root .
```

## Oxlint

The command runs `oxlint --config <file> --format json --no-error-on-unmatched-pattern [--fix] -- <paths>` with the
paths that it lints. Oxlint finds its files and applies `ignorePatterns` and `.gitignore` itself. The command adds
what Oxlint reports to its report, with the rule name that Oxlint gives (`eslint(no-var)`) and the severity of the
diagnostic.

Oxlint must run without `--type-aware`, which the command does not pass: the native linter runs the type-aware rules.
Oxlint loads every plugin of `jsPlugins`. A rule that a plugin leaves to the native linter or to the rule host must do
nothing in Oxlint, except on the `<script>` blocks of a `.vue` file for a ported rule.

A configuration that Oxlint cannot read fails the lint with what Oxlint says, as exit status 2.

## Plugins

Each entry of `jsPlugins` is a specifier, or `{ "name": "<alias>", "specifier": "<specifier>" }`.

- A specifier that starts with `./` or `../`, or an absolute path, is a file. A relative path is relative to the
  directory of the configuration file (or to `--root`).
- Any other specifier is a package, which the command resolves from the current directory.

The command loads each plugin with `import()`. The default export of the module is the plugin:

```js
export default {
    meta: { name: "my-vue", sfc: true },
    rules: { "no-v-html": { create(context) { /* ... */ } } },
};
```

The name of a plugin is its alias. Without an alias, the name is `meta.name`. Without `meta.name`, the name is the
package name. Both lose the prefix that marks a plugin, as in Oxlint: `eslint-plugin-foo` is `foo`, and
`@scope/eslint-plugin-foo` is `@scope/foo`. Two plugins with one name are an error.

The command loads every plugin of the configuration, because `meta` is only known once the module is loaded. Two
keys of `meta` give rules to the command. Oxlint runs every other rule of a plugin.

| key | meaning |
| --- | --- |
| `sfc: true` | The rules of the plugin read a whole `.vue` file. The rule host runs them on the `.vue` files ([Hosted rules](#hosted-rules)). |
| `native: ["camelcase"]` | The native linter has a port of these rules of the plugin. It runs them on the script files ([Native rules](#native-rules)). |

### Hosted rules

For a `.vue` file, the rule host runs each rule `<plugin name>/<rule>` of a plugin with `meta.sfc: true` whose entry
is not off for the file. An entry that names a rule that the plugin does not have is an error.

Oxlint loads the same plugins from the same configuration, and runs a rule that is on for a `.vue` file on the
`<script>` blocks, with a context of its own that has no template. A rule of an `sfc` plugin must do nothing there.
`context.fileRules` tells the two hosts apart: only this one sets it.

The host parses the file with `vue-eslint-parser`, and gives `@typescript-eslint/parser` to it for the `<script>`
blocks and the template expressions. A rule gets the part of ESLint's rule API that template rules use
(`npm/tsrs/src/tsrs/lint/host/types.ts` declares it):

- `context.id`, `context.cwd` (the current directory), `context.filename`, `context.physicalFilename`,
  `context.options`, `context.settings`, `context.languageOptions`, `context.sourceCode` and `context.report`.
- `context.fileRules`, which ESLint does not have: a `ReadonlyMap<string, unknown>` of every rule entry that the
  configuration resolves for the file, by the name the entry has in the configuration. It includes the rules that
  the host does not run. A template rule reads there what the script is held to, the selectors of
  `no-restricted-syntax` for instance.
- `context.sourceCode.parserServices`, with the services of `vue-eslint-parser` (`defineTemplateBodyVisitor`,
  `defineDocumentVisitor`, `getTemplateBodyTokenStore`, `getDocumentFragment`) and `defineCustomBlocksVisitor`
  for a block such as `<i18n>`.
- `report` takes `node` or `loc`, `message` or `messageId` with `data`, and `fix`. A rule that returns a fix must
  set `meta.fixable`. Suggestions are ignored.

`--fix` applies the fixes of a file, lints the result again, and repeats while a pass fixes something, up to 10
passes. A fix that overlaps a fix already applied waits for the next pass.

The host differs from ESLint in these ways:

- A listener of the script tree is a node type, or `Parent > *`, with or without `:exit`. The host rejects any other
  selector there. The listeners of `defineTemplateBodyVisitor` take every esquery selector: the parser calls them.
- No comment disables a rule.
- A syntax error in a `<script>` block is the only report of the file, as `Parsing error: ...` with the rule `vue`.
- The host reports every message, in a `<script>` block too. A rule decides what it reports.

## Native rules

The native linter is `tsrs-lint headless`. `tsrs-lint rules` prints the name of each rule that it implements: 61
rules of typescript-eslint, by their name without a prefix (`no-floating-promises`), and the ported rules, by the
name of their plugin (`js/camelcase`).

The command sends every file to one `tsrs-lint headless --runExternalCode` process, with `--project <tsconfig>` when
the option is given. Files with the same rules and options go in one group of the payload. A program is loaded once
for the script files and the `.vue` files that it holds.

### Rules of typescript-eslint

For a script file or a `.vue` file, the command takes the entries named `typescript/<name>` or
`@typescript-eslint/<name>` that are not off and whose `<name>` the native linter implements. The first option of an
entry is the options of the rule. A rule of typescript-eslint that the native linter does not implement is Oxlint's.

Do not run Oxlint with `--type-aware` on the same files: Oxlint would run the same rules a second time, through
`tsrs-lint headless`.

### Ported rules

A plugin names in `meta.native` the rules that the native linter has a port of:

```js
export default {
    meta: { name: "js", native: ["camelcase"] },
    rules: { camelcase: { create: context => (context.filename.endsWith(".vue") ? listeners(context) : {}) } },
};
```

For a script file, the command takes the entries of these rules that are not off. The native linter knows a ported
rule as `<plugin>/<rule>`, where `<plugin>` is the `meta.name` of the plugin, whatever alias the configuration gives
it. The options of the rule are the array of what follows the severity in the entry, as ESLint gives them to a
rule. An entry whose rule the native linter does not implement is an error: nothing would run the rule.

The plugin and the native linter share the work:

- The rule of the plugin must do nothing on a script file. Oxlint still loads the plugin and calls the rule there.
- The rule of the plugin still lints the `<script>` blocks of a `.vue` file, in Oxlint. The native linter sees a
  `.vue` file as what the content mapper makes of it, and the command sends it no ported rule for that file.

The ported rules are in `crates/tsrs_lint/src/ported`. Each one reports what the rule of the `js` plugin of
[revolugo-lint-plugins](https://github.com/Revolugo/revolugo-lint-plugins) reports, at the same place and with the
same message.

| rule | port of |
| --- | --- |
| `js/camelcase` | ESLint's `camelcase` |
| `js/import-export` | `export` of eslint-plugin-import-x |
| `js/no-invalid-this` | ESLint's `no-invalid-this` |

### `.vue` files

The native linter must find each `.vue` file in a tsconfig that has a content mapper for `.vue`. Without a mapper,
it reads the file as TypeScript, and the reports mean nothing. `--project` is for a repository whose type-check
projects are not the tsconfigs nearest to the sources
([tools/tsgolint/README.md](../tools/tsgolint/README.md#content-mapped-files-and---project-revolugo-fork)). With
`--project`, a script file is also linted in the project of the solution that has it, so a rule sees the types that
the type check sees.

In a `.vue` file, the command reports a diagnostic only when the diagnostic starts in a `<script>` block, as
`@vue/compiler-sfc` delimits the blocks. A Vue content mapper also maps the template expressions, so the type check
covers them. The rules are not written for them: `@click="save"` with an async `save` is the Vue idiom, and
`no-misused-promises` reports it.

### Reports and fixes

A diagnostic of a rule of typescript-eslint has the rule name `typescript-eslint(<name>)`. A diagnostic of a ported
rule has the name that Oxlint gave the rule before: `<plugin>(<rule>)`, with the name that the configuration has for
the plugin. Both have the severity of the entry. Any other diagnostic, a
tsconfig or mapper error for instance, is an error at line 1, column 1, named `typescript(<code>)`.

No comment disables a rule.

`--fix` applies the fixes of the native linter to the script files, lints the changed files again, and repeats
while a pass fixes something, up to 10 passes. A fix that touches a fix already applied waits for the next pass. The
command does not apply these fixes to a `.vue` file.

With `--fix`, the three parts run one after the other, because each one rewrites files that the next one reads:
Oxlint first, then the rule host, then the native linter.

The command finds the `tsrs` binary as the other launchers of the package do: `TSRS_BINARY`, or else the platform
package.

## Output and exit status

`unix` prints one line for each problem, sorted by file, line and column, then a summary:

```
src/App.vue:6:6: 'v-html' directive can lead to XSS attack. [Warning/my-vue/no-v-html]
src/App.vue:12:1: Promises must be awaited, add void operator to ignore. [Error/typescript-eslint(no-floating-promises)]
src/load.ts:3:7: Identifier 'page_size' is not in camel case. [Warning/js(camelcase)]
src/load.ts:8:1: Unexpected var, use let or const instead. [Error/eslint(no-var)]
Found 4 problems in 214 files.
```

`json` prints the report of ESLint's `json` formatter, with the files that have a problem:

```json
[{ "filePath": "/abs/src/App.vue", "messages": [{ "line": 6, "column": 6, "message": "...", "ruleId": "my-vue/no-v-html", "severity": 1 }] }]
```

| exit status | meaning |
| --- | --- |
| 0 | No problem. |
| 1 | At least one problem. A warning is a problem. |
| 2 | The lint did not run: a usage, configuration or plugin error, a rule that threw, or an Oxlint or `tsrs-lint` failure. The reason is on stderr, and stdout is empty. |

## Tests

```sh
vp install --frozen-lockfile
cargo build --profile dev -p tsrs_cli
(cd npm/tsrs && TSRS_BINARY=$PWD/../../target/debug/tsrs node --conditions @typescript/source --test 'test/tsrs/lint-*.test.ts')
cargo test -p tsrs_lint --test integration ported
```

The tests take the parsers from `npm/lint-vue-project`, a private workspace package that stands for a linted
project. The package is apart from `npm/package.json` because `@typescript-eslint/parser` needs the TypeScript 6 API
under the name `typescript`, which in `npm/package.json` is the TypeScript 7 build compiler. Without `TSRS_BINARY`,
the tests of the native rules are skipped.

The tests of a ported rule are in `crates/tsrs_lint/tests/integration/ported`.

`node --conditions @typescript/source npm/tsrs/bin/tsrs-lint` runs the command from its sources.
