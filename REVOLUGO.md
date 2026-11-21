# The Revolugo fork of tsrs

This repository is `oxc-project/tsrs` with three additions on the `revolugo` branch: two to its type-aware linter,
`tsrs-lint`, and a command that lints `.vue` files with it. The Revolugo monorepo type-checks and lints its
TypeScript and Vue code with this build alone.

## What the fork adds

The first two are described in
[tools/tsgolint/README.md](tools/tsgolint/README.md#content-mapped-files-and---project-revolugo-fork), the third in
[docs/LINT_VUE.md](docs/LINT_VUE.md).

- **Rules on content-mapped files.** Upstream type-checks a `.vue` file through a tsconfig `contentMappers` entry, and
  runs no lint rule on it. The fork runs the rules and reports a diagnostic at its place in the `.vue` file, when it is
  about text the file has.
- **`--project <tsconfig>`** (`TSRS_LINT_PROJECT`). A run lints each file in the project of the named solution that has
  it, before the tsconfig nearest to the file. The monorepo's type-check projects, which add the Vue content mapper,
  are not the tsconfigs next to its sources.
- **`tsrs-lint-vue`**, a command of the `tsrs` npm package. Oxlint lints the `<script>` blocks of a `.vue` file,
  and sends no `.vue` file to `tsrs-lint`. The command reads the same Oxlint configuration file and lints the rest:
  it runs the rules of the JS plugins that declare `meta.sfc: true` on the `<template>` and the custom blocks, and
  it sends the `.vue` files to `tsrs-lint` for the type-aware rules. It holds no rule and no configuration of its
  own. `tsrs-lint rules` lists the rules that `tsrs-lint` implements, which tells the command what to send.

The linter changes are in `crates/tsrs_lint` (`engine.rs`, `cli.rs`, `registry.rs`), with tests in
`crates/tsrs_lint/tests/integration`: `headless.rs`
(`rules_run_on_content_mapped_files_at_their_original_ranges`,
`project_lints_a_file_in_the_named_solution_before_its_nearest_tsconfig`) and `rules_command.rs`.

The command is in `npm/tsrs`: `bin/tsrs-lint-vue`, `lib/tsrs-lint-vue.js` and `src/tsrs/lintVue`, which
`npm/build.mjs` compiles with the JS API. Its tests are `npm/tsrs/test/tsrs/lint-vue-*.test.ts`; they take their
parsers from `npm/lint-vue-project`. `npm/sdk/sync-upstream.mjs` leaves `src/tsrs` and `test/tsrs` alone when it
copies the upstream JS API.

## Following upstream

`revolugo` is based on an upstream release tag. To move to a newer one, rebase the branch on that tag, run
`cargo test -p tsrs_lint` and the tests of the command ([docs/LINT_VUE.md](docs/LINT_VUE.md#tests)), and release.

Drop a change when upstream has it: rules on content-mapped files are tracked upstream with oxc-project/tsrs#876.

## Releasing

Run the **Revolugo release** workflow (`.github/workflows/revolugo-release.yml`) on `revolugo` with a tag named after
the upstream release and a fork revision, `v7.1.0-beta.tsrs3-revolugo.1` for instance. It builds the binaries for
macOS arm64 and Linux (x64 and arm64, glibc and musl), packs the `tsrs` npm package and its platform packages as
upstream does, and attaches the tarballs to a GitHub release. Nothing goes to npm.

The packages keep upstream's names, so that Oxlint finds `tsrs-lint` and the launcher finds its platform package.
Their version is the tag's (`7.1.0-beta.tsrs3-revolugo.1`), which the npm registry does not have: a platform package
can only come from the release.

A consumer installs the main package by URL. pnpm takes no URL for a subdependency (`blockExoticSubdeps`), so the
platform packages are installed as optional dependencies of the consumer's root package, where the launcher finds
them, and the ones the main package names are removed (`package.json`, then `pnpm-workspace.yaml`):

```json
{
  "devDependencies": {
    "tsrs": "https://github.com/Revolugo/revolugo-tsrs/releases/download/<tag>/tsrs-<version>.tgz"
  },
  "optionalDependencies": {
    "@tsrs/darwin-arm64": "https://github.com/Revolugo/revolugo-tsrs/releases/download/<tag>/tsrs-darwin-arm64-<version>.tgz",
    "@tsrs/linux-x64-gnu": "https://github.com/Revolugo/revolugo-tsrs/releases/download/<tag>/tsrs-linux-x64-gnu-<version>.tgz"
  }
}
```

```yaml
overrides:
  'tsrs>@tsrs/darwin-arm64': '-'
  'tsrs>@tsrs/linux-x64-gnu': '-'
  # … one line per platform of the release, in both files
```
