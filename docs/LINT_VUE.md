# `tsrs-lint-vue`: lint what Oxlint cannot see of a `.vue` file (revolugo fork)

Oxlint lints the `<script>` blocks of a `.vue` file as it lints a `.ts` file. It does not parse the `<template>` or
the custom blocks, and it sends no `.vue` file to `tsrs-lint`. `tsrs-lint-vue` covers both gaps from the same Oxlint
configuration file:

- It runs the rules of the JS plugins that declare `meta.sfc: true` on the whole file, in a rule host of its own.
- It runs the type-aware rules that `tsrs-lint` implements, through the content mapper of the file's tsconfig.

The command ships in the `tsrs` npm package, next to `tsrs` and `tsrs-lint`. Its sources are in
`npm/tsrs/src/tsrs/lintVue`.

## Usage

```sh
tsrs-lint-vue [--fix] [--format unix|json] [--config <file>] [--root <dir>] [--project <tsconfig>] [paths...]
```

Run the command from the root of the project. The current directory is where the command lists the files, finds
`node_modules`, and runs `tsrs-lint`.

| option | meaning |
| --- | --- |
| `paths` | The files and directories to lint, relative to the current directory. The default is `.`. |
| `--fix` | Apply the fixes of the plugin rules and write the files. The report is about the fixed files. |
| `-f`, `--format` | `unix` (the default) or `json`. |
| `-c`, `--config` | The Oxlint configuration file. The default is `.oxlintrc.json` in the current directory. |
| `--root` | The directory that the configuration applies to. The default is the directory of the configuration file. |
| `-p`, `--project` | The tsconfig that `tsrs-lint headless --project` gets. |
| `-h`, `--help` | Print the usage. |

## Requirements

- Node.js 22.18 or later. A plugin is an ES module, and Node.js 22.18 loads one that is written in TypeScript. The
  rest of the `tsrs` package still runs on Node.js 16.20.
- A git repository. The command lists the files with `git ls-files -co --exclude-standard`: tracked files and
  untracked files that `.gitignore` does not exclude.
- `vue-eslint-parser` and `@typescript-eslint/parser` in the project, when a plugin rule is on for a file.
- `@vue/compiler-sfc` in the project, when a type-aware rule is on for a file.

The `tsrs` package depends on none of these packages. The command resolves each one from the current directory, as a
module of the project does, and names the missing package when the project does not have it. None of them needs the
`eslint` package.

## Files

The command lints the `.vue` files under `paths` that `ignorePatterns` does not match. An ignore pattern matches a
file, or a directory at any depth: `dist`, `**/dist` and `apps/web/generated` all work.

## Configuration

The command reads these keys of the configuration file. It ignores every other key, `extends` included.

| key | use |
| --- | --- |
| `rules` | The rule entries of every file. |
| `overrides[].files`, `overrides[].excludeFiles`, `overrides[].rules` | The rule entries of the files that match. |
| `jsPlugins`, `overrides[].jsPlugins` | The plugins to load. |
| `settings` | Passed to the plugin rules as `context.settings`. |
| `ignorePatterns` | The files to leave out. |

The file must be JSON without comments.

The rule entries of a file are the base `rules`, then the `rules` of each override whose `files` match the file and
whose `excludeFiles` do not, in order. A later entry replaces an earlier one. A later entry that is a severity alone
keeps the options of the earlier one, unless the severity turns the rule off. A severity is `0`, `1`, `2`, `"off"`,
`"allow"`, `"warn"`, `"error"` or `"deny"`.

Globs and relative plugin paths are resolved against the directory of the configuration file, as in Oxlint. `--root`
names another directory. Use it for a configuration file that is not in the tree that it configures, a generated one
for instance:

```sh
tsrs-lint-vue --config /tmp/generated.oxlintrc.json --root .
```

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

The command loads every plugin of the configuration, because `meta.sfc` is only known once the module is loaded. A
plugin without `meta.sfc: true` is Oxlint's business: the command runs none of its rules.

### Hosted rules

For a `.vue` file, the rule host runs each rule `<plugin name>/<rule>` of a plugin with `meta.sfc: true` whose entry
is not off for the file. An entry that names a rule that the plugin does not have is an error.

Oxlint loads the same plugins from the same configuration, and runs a rule that is on for a `.vue` file on the
`<script>` blocks, with a context of its own that has no template. A rule of an `sfc` plugin must do nothing there.
`context.fileRules` tells the two hosts apart: only this one sets it.

The host parses the file with `vue-eslint-parser`, and gives `@typescript-eslint/parser` to it for the `<script>`
blocks and the template expressions. A rule gets the part of ESLint's rule API that template rules use
(`npm/tsrs/src/tsrs/lintVue/host/types.ts` declares it):

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

The host differs from ESLint in these ways:

- A listener of the script tree is a node type, or `Parent > *`, with or without `:exit`. The host rejects any other
  selector there. The listeners of `defineTemplateBodyVisitor` take every esquery selector: the parser calls them.
- No comment disables a rule.
- A syntax error in a `<script>` block is the only report of the file, as `Parsing error: ...` with the rule `vue`.
- The host reports every message, in a `<script>` block too. A rule decides what it reports.

`--fix` applies the fixes of a file, lints the result again, and repeats while a pass fixes something, up to 10
passes. A fix that overlaps a fix already applied waits for the next pass.

## Type-aware rules

For a `.vue` file, the command takes the entries named `typescript/<name>` or `@typescript-eslint/<name>` that are
not off and whose `<name>` is in the output of `tsrs-lint rules`. It sends the files to
`tsrs-lint headless --runExternalCode`, with `--project <tsconfig>` when the option is given. Files with the same
rules and options go in one group of the payload. The first option of an entry is the options of the rule.

`tsrs-lint` must find each file in a tsconfig that has a content mapper for `.vue`. Without a mapper, `tsrs-lint`
reads the file as TypeScript, and the reports mean nothing. `--project` is for a repository whose type-check
projects are not the tsconfigs nearest to the sources
([tools/tsgolint/README.md](../tools/tsgolint/README.md#content-mapped-files-and---project-revolugo-fork)).

The command reports a diagnostic only when the diagnostic starts in a `<script>` block, as `@vue/compiler-sfc`
delimits the blocks. A Vue content mapper also maps the template expressions, so the type check covers them. The
rules are not written for them: `@click="save"` with an async `save` is the Vue idiom, and `no-misused-promises`
reports it.

A diagnostic of a rule has the rule name `typescript-eslint(<name>)` and the severity of the entry. Any other
diagnostic, a tsconfig or mapper error for instance, is an error at line 1, column 1, named `typescript(<code>)`.
`--fix` does not apply the fixes of these rules.

The command finds the `tsrs` binary as the other launchers of the package do: `TSRS_BINARY`, or else the platform
package.

## Output and exit status

`unix` prints one line for each problem, sorted by file, line and column, then a summary:

```
src/App.vue:6:6: 'v-html' directive can lead to XSS attack. [Warning/my-vue/no-v-html]
src/App.vue:12:1: Promises must be awaited, add void operator to ignore. [Error/typescript-eslint(no-floating-promises)]
Found 2 problems in 14 .vue files.
```

`json` prints the report of ESLint's `json` formatter, with the files that have a problem:

```json
[{ "filePath": "/abs/src/App.vue", "messages": [{ "line": 6, "column": 6, "message": "...", "ruleId": "my-vue/no-v-html", "severity": 1 }] }]
```

| exit status | meaning |
| --- | --- |
| 0 | No problem. |
| 1 | At least one problem. A warning is a problem. |
| 2 | The lint did not run: a usage, configuration or plugin error, a rule that threw, or a `tsrs-lint` failure. The reason is on stderr, and stdout is empty. |

## Tests

```sh
vp install --frozen-lockfile
cargo build --profile dev -p tsrs_cli
(cd npm/tsrs && TSRS_BINARY=$PWD/../../target/debug/tsrs node --conditions @typescript/source --test 'test/tsrs/lint-vue-*.test.ts')
```

The tests take the parsers from `npm/lint-vue-project`, a private workspace package that stands for a linted
project. The package is apart from `npm/package.json` because `@typescript-eslint/parser` needs the TypeScript 6 API
under the name `typescript`, which in `npm/package.json` is the TypeScript 7 build compiler. Without `TSRS_BINARY`,
the test of the type-aware rules is skipped.

`node --conditions @typescript/source npm/tsrs/bin/tsrs-lint-vue` runs the command from its sources.
