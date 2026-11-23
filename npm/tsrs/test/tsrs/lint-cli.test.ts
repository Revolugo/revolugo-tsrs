// tsrs-lint, the command: what it lints, prints and exits with. The last tests run the rules of the native linter,
// in the `tsrs` binary of TSRS_BINARY.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
    createProject,
    lint,
    lintWithOxlint,
    SFC_PLUGIN,
} from "./lint.testUtils.ts";

const APP = '<script setup lang="ts">\nold()\n</script>\n\n<template>\n  <p v-html="old()" />\n</template>\n';
const CLEAN = "<template>\n  <p>fine</p>\n</template>\n";

/** A project whose configuration hosts the rules of ./plugins/sfc.mjs on the .vue files. */
const project = (t: test.TestContext, files: Readonly<Record<string, unknown>> = {}, provided?: readonly string[]): string =>
    createProject(t, {
        ".oxlintrc.json": {
            ignorePatterns: ["generated"],
            jsPlugins: ["./plugins/sfc.mjs"],
            overrides: [
                { files: ["**/*.vue"], rules: { "sfc/no-old": "error", "sfc/no-v-html": 1 } },
                { excludeFiles: ["src/legacy/Kept.vue"], files: ["src/legacy/**"], rules: { "sfc/no-v-html": "off" } },
            ],
            rules: { "no-var": 2 },
        },
        "generated/Ignored.vue": APP,
        "plugins/sfc.mjs": SFC_PLUGIN,
        "src/App.vue": APP,
        "src/Clean.vue": CLEAN,
        "src/legacy/Kept.vue": '<template>\n  <p v-html="x" />\n</template>\n',
        "src/legacy/Old.vue": '<template>\n  <p v-html="x" />\n</template>\n',
        "src/not-vue.ts": "old()\n",
        ...files,
    }, provided);

test("the unix format has one line for each problem and a summary, and a problem fails the lint", t => {
    const root = project(t);

    assert.deepEqual(lint(root), {
        status: 1,
        stderr: "",
        stdout: [
            "src/App.vue:2:1: Rename old. [Error/sfc/no-old]",
            "src/App.vue:6:6: No v-html. [Warning/sfc/no-v-html]",
            "src/App.vue:6:14: Rename old. [Error/sfc/no-old]",
            "src/legacy/Kept.vue:2:6: No v-html. [Warning/sfc/no-v-html]",
            // The four .vue files that are not ignored, and the two script files.
            "Found 4 problems in 6 files.",
            "",
        ].join("\n"),
    });
});

test("the paths limit the lint, a warning alone fails it, and a lint without a problem succeeds", t => {
    const root = project(t);

    assert.deepEqual(lint(root, ["src/legacy"]), {
        status: 1,
        stderr: "",
        stdout: "src/legacy/Kept.vue:2:6: No v-html. [Warning/sfc/no-v-html]\nFound 1 problem in 2 files.\n",
    });
    assert.deepEqual(lint(root, ["src/Clean.vue", "src/legacy/Old.vue"]), {
        status: 0,
        stderr: "",
        stdout: "Found 0 problems in 2 files.\n",
    });
    assert.deepEqual(lint(root, ["src/Clean.vue"]).stdout, "Found 0 problems in 1 file.\n");
    // An ignored file is not linted, even when it is named.
    assert.deepEqual(lint(root, ["generated"]).stdout, "Found 0 problems in 0 files.\n");
});

test("a file that .gitignore names is not linted, tracked or not, as in Oxlint", t => {
    const root = project(t, { "src/Tracked.vue": APP, "src/Untracked.vue": APP });
    execFileSync("git", ["add", "src/Tracked.vue"], { cwd: root });
    fs.appendFileSync(path.join(root, ".gitignore"), "*racked.vue\n");

    assert.deepEqual(lint(root, ["src/Tracked.vue", "src/Untracked.vue", "src/Clean.vue"]).stdout, "Found 0 problems in 1 file.\n");
});

test("the json format is ESLint's report", t => {
    const root = project(t);
    const run = lint(root, ["--format", "json", "src/App.vue", "src/Clean.vue", "src/legacy"]);

    assert.equal(run.status, 1);
    assert.deepEqual(JSON.parse(run.stdout), [
        {
            filePath: path.join(root, "src/App.vue"),
            messages: [
                { column: 1, line: 2, message: "Rename old.", ruleId: "sfc/no-old", severity: 2 },
                { column: 6, line: 6, message: "No v-html.", ruleId: "sfc/no-v-html", severity: 1 },
                { column: 14, line: 6, message: "Rename old.", ruleId: "sfc/no-old", severity: 2 },
            ],
        },
        {
            filePath: path.join(root, "src/legacy/Kept.vue"),
            messages: [{ column: 6, line: 2, message: "No v-html.", ruleId: "sfc/no-v-html", severity: 1 }],
        },
    ]);
    assert.deepEqual(lint(root, ["-f", "json", "src/Clean.vue"]), { status: 0, stderr: "", stdout: "[]\n" });
});

test("--fix writes the fixed files and reports what is left", t => {
    const root = project(t);

    assert.deepEqual(lint(root, ["--fix", "src/App.vue", "src/Clean.vue"]), {
        status: 1,
        stderr: "",
        stdout: "src/App.vue:6:6: No v-html. [Warning/sfc/no-v-html]\nFound 1 problem in 2 files.\n",
    });
    assert.equal(fs.readFileSync(path.join(root, "src/App.vue"), "utf8"), APP.replaceAll("old()", "renamed()"));
    assert.equal(fs.readFileSync(path.join(root, "src/Clean.vue"), "utf8"), CLEAN);
    assert.equal(fs.readFileSync(path.join(root, "src/not-vue.ts"), "utf8"), "old()\n");
});

test("a syntax error of a script is reported without a rule", t => {
    const root = project(t, { "src/Broken.vue": '<script setup lang="ts">\nconst = 1\n</script>\n' });
    const run = lint(root, ["src/Broken.vue"]);

    assert.equal(run.status, 1);
    assert.match(run.stdout, /^src\/Broken\.vue:2:\d+: Parsing error: .* \[Error\/vue\]\nFound 1 problem in 1 file\.\n$/u);
});

test("--config names a configuration elsewhere, and --root the directory it applies to", t => {
    const root = project(t, {
        "config/elsewhere.json": {
            jsPlugins: [{ name: "renamed", specifier: "./plugins/sfc.mjs" }],
            overrides: [{ files: ["src/*.vue"], rules: { "renamed/no-v-html": 2 } }],
        },
    });

    // Without --root, the globs and the plugin path are relative to config/: the plugin is not there.
    const relative = lint(root, ["--config", "config/elsewhere.json"]);
    assert.equal(relative.status, 2);
    assert.match(relative.stderr, /^tsrs-lint: Cannot load the plugin \.\/plugins\/sfc\.mjs \(file:.*\/config\/plugins\/sfc\.mjs\)/u);
    assert.deepEqual(lint(root, ["-c", "config/elsewhere.json", "--root", "."]), {
        status: 1,
        stderr: "",
        stdout: "src/App.vue:6:6: No v-html. [Error/renamed/no-v-html]\nFound 1 problem in 7 files.\n",
    });
});

test("a lint that cannot run exits with 2 and says why", t => {
    const root = project(t, {
        "bad-rule.json": { jsPlugins: ["./plugins/sfc.mjs"], rules: { "sfc/no-such-rule": 2 } },
        "bad-severity.json": { jsPlugins: ["./plugins/sfc.mjs"], rules: { "sfc/no-old": "loud" } },
    });
    const failure = (args: readonly string[], cwd = root): string => {
        const run = lint(cwd, args);
        assert.deepEqual({ status: run.status, stdout: run.stdout }, { status: 2, stdout: "" }, run.stderr);
        return run.stderr;
    };

    assert.match(failure(["--unknown"]), /^tsrs-lint: Unknown option '--unknown'.*\nRun tsrs-lint --help for the usage\.\n$/su);
    assert.equal(failure(["--format", "stylish"]), "tsrs-lint: --format is unix or json, not stylish\nRun tsrs-lint --help for the usage.\n");
    assert.match(failure(["--config", "missing.json"]), /^tsrs-lint: Cannot read the configuration .*missing\.json: ENOENT/u);
    assert.equal(failure(["--config", "bad-rule.json"]), "tsrs-lint: No plugin has a rule named sfc/no-such-rule\n");
    assert.match(failure(["--config", "bad-severity.json"]), /^tsrs-lint: The severity of sfc\/no-old is not one of/u);

    // A project without the parsers: the error names the package to install.
    const bare = project(t, {}, []);
    assert.match(
        failure([], bare),
        /^tsrs-lint: Cannot find the package vue-eslint-parser from .*: add vue-eslint-parser to the project's dependencies\.\n$/u,
    );
    const half = project(t, {}, ["vue-eslint-parser"]);
    assert.match(failure([], half), /^tsrs-lint: Cannot find the package @typescript-eslint\/parser from /u);

    // Not a git repository: the files cannot be listed.
    fs.rmSync(path.join(bare, ".git"), { recursive: true });
    assert.match(failure([], bare), /^tsrs-lint: Cannot list the files of .* with git ls-files: /u);
});

test("a rule that throws fails the lint with the rule, the file and the stack", t => {
    const root = project(t, {
        ".oxlintrc.json": { jsPlugins: ["./plugins/throws.mjs"], rules: { "throws/always": 2 } },
        "plugins/throws.mjs": 'export default { meta: { name: "throws", sfc: true }, rules: { always: { create() { throw new TypeError("boom"); } } } };\n',
    });
    const run = lint(root, ["src/Clean.vue"]);

    assert.equal(run.status, 2);
    assert.equal(run.stdout, "");
    assert.match(run.stderr, /^tsrs-lint: Error: throws\/always on .*\/src\/Clean\.vue: boom\n/u);
    assert.match(run.stderr, /\[cause\]: TypeError: boom\n\s+at .*create.*plugins\/throws\.mjs/u);
});

test("--help prints the usage, with the Node.js version the command needs", t => {
    const run = lint(project(t), ["--help"]);

    assert.equal(run.status, 0);
    assert.equal(run.stderr, "");
    assert.match(run.stdout, /^Usage: tsrs-lint \[options\] \[paths\.\.\.\]\n/u);
    assert.match(run.stdout, /Node\.js 22\.18 or later/u);
    for (const option of ["--fix", "--format", "--config", "--root", "--project"]) {
        assert.ok(run.stdout.includes(option), option);
    }
});

// The content mapper of testdata/contentmapper checks a whole .vue file as TypeScript. This one keeps its protocol
// and checks what a Vue mapper does: the `<script>` block, then the expression of the first `{{ }}` of the template,
// each mapped to its place in the file.
const HEADER_MAPPER = fileURLToPath(new URL("../../../../testdata/contentmapper/header-mapper/", import.meta.url));
const TRANSFORM = /function transform\(params\) \{.*?\n\}\n/su;
const VUE_TRANSFORM = `function transform(params) {
  // The offsets of a mapping are UTF-8 bytes.
  const content = Buffer.from(params.content)
  const header = Buffer.from(HEADER)
  const scriptStart = content.indexOf('>', content.indexOf('<script')) + 1
  const script = content.subarray(scriptStart, content.indexOf('</script>'))
  const expressionStart = content.indexOf('{{') + 2
  const expression = content.subarray(expressionStart, content.indexOf('}}'))
  return {
    text: Buffer.concat([header, script, Buffer.from(';\\n'), expression, Buffer.from('\\n')]).toString(),
    extension: '.ts',
    mappings: [
      [header.length, script.length, scriptStart, script.length, 0],
      [header.length + script.length + 2, expression.length, expressionStart, expression.length, 0],
    ],
    diagnostics: [],
  }
}
`;

function vueMapper(): string {
    const mapper = fs.readFileSync(path.join(HEADER_MAPPER, "mapper.mjs"), "utf8");
    assert.match(mapper, TRANSFORM, "the transform of header-mapper changed");
    return mapper.replace(TRANSFORM, () => VUE_TRANSFORM);
}

const NEEDS_BINARY = { skip: process.env["TSRS_BINARY"] ? false : "needs the tsrs binary of TSRS_BINARY" };

const TYPED = `<script setup lang="ts">
async function load(): Promise<number> {
  return 1
}
load()
const text: string = 'é'; load()
await text
</script>

<template>
  <p @click="load">{{ load() }}</p>
</template>
`;

test("the rules of typescript-eslint run on a .vue file through the content mapper, and report in the <script> blocks", NEEDS_BINARY, t => {
    const root = project(t, {
        ".oxlintrc.json": {
            overrides: [
                { files: ["**/*.vue"], rules: { "@typescript-eslint/await-thenable": "warn", "typescript/no-floating-promises": [2, { ignoreVoid: true }] } },
                { files: ["src/Clean.vue"], rules: { "typescript/no-floating-promises": "off" } },
            ],
            // A rule of typescript-eslint that tsrs-lint does not have is Oxlint's, not ours.
            rules: { "typescript/no-explicit-any": 2, "typescript/no-floating-promises": 2 },
        },
        "check/tsconfig.json": {
            compilerOptions: { module: "esnext", noEmit: true, strict: true, target: "es2022", types: [] },
            contentMappers: [{ extensions: [".vue"], package: "header-mapper" }],
            include: ["../src"],
        },
        "node_modules/header-mapper/mapper.mjs": vueMapper(),
        "node_modules/header-mapper/package.json": fs.readFileSync(path.join(HEADER_MAPPER, "package.json"), "utf8"),
        "src/Typed.vue": TYPED,
    });
    const run = lint(root, ["--project", "check/tsconfig.json", "src/Typed.vue", "src/Clean.vue"]);

    // `{{ load() }}` floats a promise too, and tsrs-lint reports it: a template expression is type-checked, not
    // linted. The column of the second report is in characters: tsrs-lint counts the `é` before it as two bytes.
    assert.equal(run.stderr, "");
    assert.deepEqual(run.stdout.split("\n").map(line => line.replace(/: .* \[/u, " [")), [
        "src/Typed.vue:5:1 [Error/typescript-eslint(no-floating-promises)]",
        "src/Typed.vue:6:27 [Error/typescript-eslint(no-floating-promises)]",
        "src/Typed.vue:7:1 [Warning/typescript-eslint(await-thenable)]",
        "Found 3 problems in 2 files.",
        "",
    ]);
    assert.match(run.stdout, /^src\/Typed\.vue:5:1: Promises must be awaited/u);
    assert.equal(run.status, 1);

    // The tsconfig with the content mapper is not the one nearest to the file: without --project, tsrs-lint does not
    // see the floating promises.
    assert.doesNotMatch(lint(root, ["src/Typed.vue"]).stdout, /no-floating-promises/u);
});

// Oxlint runs `other`, and leaves `camelcase` to tsrs-lint, which has a port of it under the plugin's own name.
// Oxlint takes the options of a rule that declares a schema.
const NATIVE_PLUGIN = 'export default { meta: { name: "js", native: ["camelcase"] }, rules: { camelcase: { meta: { schema: [{ type: "object" }] }, create: () => ({}) }, other: { create: () => ({}) } } };\n';

const SCRIPT = `async function load(): Promise<number> {
  return 1
}
const snake_case = { other_key: 'é' }; load()
export const value = snake_case.other_key as string
`;

/** A project whose configuration gives its script files rules of typescript-eslint and a rule that a plugin leaves to tsrs-lint. */
const scripts = (t: test.TestContext, files: Readonly<Record<string, unknown>> = {}): string =>
    createProject(t, {
        ".oxlintrc.json": {
            // The configuration names the rules of the plugin with an alias.
            jsPlugins: [{ name: "core", specifier: "./plugins/js.mjs" }],
            overrides: [{ files: ["**/*.test.ts"], rules: { "core/camelcase": "off" } }],
            rules: {
                "core/camelcase": [1, { properties: "never" }],
                "core/other": 2,
                "typescript/no-floating-promises": 2,
                "typescript/no-unnecessary-type-assertion": 2,
            },
        },
        "plugins/js.mjs": NATIVE_PLUGIN,
        "src/a.test.ts": "export const in_a_test = 1\n",
        "src/a.ts": SCRIPT,
        // tsrs-lint sees a .vue file as what the content mapper makes of it: Oxlint runs the JS rule on its script.
        "src/Snake.vue": '<script setup lang="ts">\nconst snake_case = 1\n</script>\n',
        "tsconfig.json": { compilerOptions: { module: "esnext", noEmit: true, strict: true, target: "es2022", types: [] }, include: ["src"] },
        ...files,
    });

test("the rules that tsrs-lint implements run on the script files: those of typescript-eslint, and those that a plugin leaves to it", NEEDS_BINARY, t => {
    const root = scripts(t);
    const run = lint(root);

    // The columns are in characters: tsrs-lint counts the `é` as two bytes.
    assert.equal(run.stderr, "");
    assert.deepEqual(run.stdout.split("\n").map(line => line.replace(/: .* \[/u, " [")), [
        "src/a.ts:4:7 [Warning/core(camelcase)]",
        "src/a.ts:4:40 [Error/typescript-eslint(no-floating-promises)]",
        // Two rules report at one place: the assertion, and the name it starts with.
        "src/a.ts:5:22 [Error/typescript-eslint(no-unnecessary-type-assertion)]",
        "src/a.ts:5:22 [Warning/core(camelcase)]",
        "Found 4 problems in 4 files.",
        "",
    ]);
    assert.match(run.stdout, /^src\/a\.ts:4:7: Identifier 'snake_case' is not in camel case\. \[Warning\/core\(camelcase\)\]$/mu);
    assert.equal(run.status, 1);
});

test("--fix applies the fixes of tsrs-lint to the script files, and reports what is left", NEEDS_BINARY, t => {
    const root = scripts(t);
    const run = lint(root, ["--fix", "--format", "json", "src/a.ts"]);

    assert.equal(run.stderr, "");
    assert.deepEqual(
        JSON.parse(run.stdout).flatMap((file: { messages: { ruleId: string; }[]; }) => file.messages.map(message => message.ruleId)),
        ["core(camelcase)", "typescript-eslint(no-floating-promises)", "core(camelcase)"],
    );
    assert.equal(fs.readFileSync(path.join(root, "src/a.ts"), "utf8"), SCRIPT.replace(" as string", ""));
});

test("a rule that a plugin leaves to tsrs-lint, and that tsrs-lint does not have, fails the lint", NEEDS_BINARY, t => {
    const root = scripts(t, { "plugins/js.mjs": NATIVE_PLUGIN.replace('"camelcase"', '"camelcase", "other"') });
    const run = lint(root);

    assert.deepEqual({ status: run.status, stdout: run.stdout }, { status: 2, stdout: "" });
    assert.equal(
        run.stderr,
        "tsrs-lint: The plugin of core/other leaves the rule to tsrs-lint (meta.native), and this tsrs-lint has no rule js/other: `tsrs-lint rules` lists those it has\n",
    );
});

test("tsrs-lint headless and tsrs-lint rules are the native linter", NEEDS_BINARY, t => {
    const run = lintWithOxlint(project(t), ["rules"]);

    assert.equal(run.status, 0);
    assert.ok(run.stdout.split("\n").includes("no-floating-promises"), run.stdout);
});

/** The configuration of `scripts`, with a rule of Oxlint and the rules of its correctness category off. */
const WITH_OXLINT = {
    categories: { correctness: "off" },
    jsPlugins: [{ name: "core", specifier: "./plugins/js.mjs" }],
    overrides: [{ files: ["**/*.test.ts"], rules: { "core/camelcase": "off" } }],
    rules: {
        "core/camelcase": [1, { properties: "never" }],
        "no-var": 2,
        "typescript/no-floating-promises": 2,
    },
};

test("Oxlint runs with the rest, and the lint prints one report of both", NEEDS_BINARY, t => {
    const root = scripts(t, { ".oxlintrc.json": WITH_OXLINT, "src/b.ts": "var legacy = 1\nexport { legacy }\n" });
    const run = lintWithOxlint(root);

    assert.equal(run.stderr, "");
    assert.deepEqual(run.stdout.split("\n").map(line => line.replace(/: .* \[/u, " [")), [
        "src/a.ts:4:7 [Warning/core(camelcase)]",
        "src/a.ts:4:40 [Error/typescript-eslint(no-floating-promises)]",
        "src/a.ts:5:22 [Warning/core(camelcase)]",
        "src/b.ts:1:1 [Error/eslint(no-var)]",
        "Found 4 problems in 5 files.",
        "",
    ]);
    assert.equal(run.status, 1);
    // A JS rule that Oxlint runs on the script of a .vue file is in the report too: `camelcase` of the plugin does
    // nothing here.
    assert.equal(lintWithOxlint(root, ["src/Snake.vue"]).stdout, "Found 0 problems in 1 file.\n");
});

test("--fix applies the fixes of Oxlint first, then those of tsrs-lint", NEEDS_BINARY, t => {
    const root = scripts(t, {
        ".oxlintrc.json": { ...WITH_OXLINT, rules: { ...WITH_OXLINT.rules, "typescript/no-unnecessary-type-assertion": 2 } },
        "src/b.ts": "var legacy: string = 'x' as string\nexport { legacy }\n",
    });
    const run = lintWithOxlint(root, ["--fix", "src/b.ts"]);

    assert.deepEqual(run, { status: 0, stderr: "", stdout: "Found 0 problems in 1 file.\n" });
    assert.equal(fs.readFileSync(path.join(root, "src/b.ts"), "utf8"), "const legacy: string = 'x'\nexport { legacy }\n");
});

test("a lint with Oxlint needs the oxlint package of the project, and takes no --root", t => {
    const root = createProject(t, { ".oxlintrc.json": { rules: { "no-var": 2 } }, "src/a.ts": "var a = 1\n" }, []);
    const missing = lintWithOxlint(root);
    assert.deepEqual({ status: missing.status, stdout: missing.stdout }, { status: 2, stdout: "" });
    assert.match(missing.stderr, /^tsrs-lint: Cannot find the package oxlint from .*: add oxlint to the project's dependencies, or pass --no-oxlint\.\n$/u);
    assert.equal(lint(root).stdout, "Found 0 problems in 1 file.\n");

    const rooted = lintWithOxlint(root, ["--root", "."]);
    assert.equal(rooted.status, 2);
    assert.match(rooted.stderr, /^tsrs-lint: --root needs --no-oxlint: /u);
});

test("a configuration that Oxlint cannot read fails the lint with what Oxlint says", t => {
    const root = createProject(t, { ".oxlintrc.json": { rules: { "nope/rule": 2 } }, "src/a.ts": "export const a = 1\n" }, ["oxlint"]);
    const run = lintWithOxlint(root);

    assert.deepEqual({ status: run.status, stdout: run.stdout }, { status: 2, stdout: "" });
    assert.match(run.stderr, /oxlint failed \(exit 1\): .*Plugin 'nope' not found/su);
});
