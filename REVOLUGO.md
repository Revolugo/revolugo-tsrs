# The Revolugo fork of tsrs

This repository is `oxc-project/tsrs` with two additions to its type-aware linter, `tsrs-lint`, on the `revolugo`
branch. The Revolugo monorepo type-checks and lints its TypeScript and Vue code with this build alone.

## What the fork adds

Both are described in [tools/tsgolint/README.md](tools/tsgolint/README.md#content-mapped-files-and---project-revolugo-fork).

- **Rules on content-mapped files.** Upstream type-checks a `.vue` file through a tsconfig `contentMappers` entry, and
  runs no lint rule on it. The fork runs the rules and reports a diagnostic at its place in the `.vue` file, when it is
  about text the file has.
- **`--project <tsconfig>`** (`TSRS_LINT_PROJECT`). A run lints each file in the project of the named solution that has
  it, before the tsconfig nearest to the file. The monorepo's type-check projects, which add the Vue content mapper,
  are not the tsconfigs next to its sources.

Oxlint sends no `.vue` file to `tsrs-lint`, so the monorepo writes that payload itself (`tools/lint/vue.ts` there).

The changes are in `crates/tsrs_lint` (`engine.rs`, `cli.rs`), with tests in
`crates/tsrs_lint/tests/integration/headless.rs`
(`rules_run_on_content_mapped_files_at_their_original_ranges`,
`project_lints_a_file_in_the_named_solution_before_its_nearest_tsconfig`).

## Following upstream

`revolugo` is based on an upstream release tag. To move to a newer one, rebase the branch on that tag, run
`cargo test -p tsrs_lint`, and release.

Drop a change when upstream has it: rules on content-mapped files are tracked upstream with oxc-project/tsrs#876.

## Releasing

Run the **Revolugo release** workflow (`.github/workflows/revolugo-release.yml`) on `revolugo` with a tag named after
the upstream release and a fork revision, `v7.1.0-beta.tsrs3-revolugo.1` for instance. It builds the binaries for
macOS arm64 and Linux (x64 and arm64, glibc and musl), packs the `tsrs` npm package and its platform packages as
upstream does, and attaches the tarballs to a GitHub release. Nothing goes to npm.

The packages keep upstream's names, so that Oxlint finds `tsrs-lint` and the launcher finds its platform package.
Their version is the tag's (`7.1.0-beta.tsrs3-revolugo.1`), which the npm registry does not have: a platform package
can only come from the release. A consumer installs them by URL and overrides the platform packages that the main
one names (pnpm, `pnpm-workspace.yaml`):

```yaml
overrides:
  tsrs: https://github.com/revolugo/revolugo-tsrs/releases/download/<tag>/tsrs-<version>.tgz
  '@tsrs/darwin-arm64': https://github.com/revolugo/revolugo-tsrs/releases/download/<tag>/tsrs-darwin-arm64-<version>.tgz
  '@tsrs/linux-x64-gnu': https://github.com/revolugo/revolugo-tsrs/releases/download/<tag>/tsrs-linux-x64-gnu-<version>.tgz
  # … one line per platform of the release
```
