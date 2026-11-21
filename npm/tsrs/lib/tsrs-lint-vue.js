// `tsrs-lint-vue`: lints what Oxlint cannot see of a .vue file (docs/LINT_VUE.md). The command is compiled from
// src/tsrs/lintVue; `node --conditions @typescript/source bin/tsrs-lint-vue` runs its sources.
import { main } from "#lintVue";

process.exitCode = await main(process.argv.slice(2));
