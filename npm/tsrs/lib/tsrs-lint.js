// `tsrs-lint`: the linter.
//
// `tsrs-lint headless ...` and `tsrs-lint rules` are `tsrs --lint`, the native linter: oxlint runs it as
// `<path> headless ...` (OXLINT_TSGOLINT_PATH=node_modules/.bin/tsrs-lint).
//
// Anything else is the lint of a project (revolugo fork, docs/LINT.md): it reads an Oxlint configuration and runs
// the native linter itself. The command is compiled from src/tsrs/lint; `node --conditions @typescript/source
// bin/tsrs-lint` runs its sources.
const NATIVE = new Set(["headless", "--headless", "rules"]);

if (NATIVE.has(process.argv[2])) {
    process.argv.splice(2, 0, "--lint");
    await import("./tsrs.js");
}
else {
    const [major, minor] = process.versions.node.split(".").map(Number);
    if (major < 22 || (major === 22 && minor < 18)) {
        console.error(`tsrs-lint requires Node.js 22.18 or later (this is ${process.versions.node}): it loads plugins that are ES modules, written in TypeScript too.`);
        process.exit(2);
    }
    const { main } = await import("#lint");
    process.exitCode = await main(process.argv.slice(2));
}
