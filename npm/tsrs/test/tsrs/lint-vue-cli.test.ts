// tsrs-lint-vue, the command: what it lints, prints and exits with. The last test runs the type-aware rules, in the
// `tsrs` binary of TSRS_BINARY.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import {
    createProject,
    lintVue,
    SFC_PLUGIN,
} from "./lint-vue.testUtils.ts";

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

    assert.deepEqual(lintVue(root), {
        status: 1,
        stderr: "",
        stdout: [
            "src/App.vue:2:1: Rename old. [Error/sfc/no-old]",
            "src/App.vue:6:6: No v-html. [Warning/sfc/no-v-html]",
            "src/App.vue:6:14: Rename old. [Error/sfc/no-old]",
            "src/legacy/Kept.vue:2:6: No v-html. [Warning/sfc/no-v-html]",
            "Found 4 problems in 4 .vue files.",
            "",
        ].join("\n"),
    });
});

test("the paths limit the lint, a warning alone fails it, and a lint without a problem succeeds", t => {
    const root = project(t);

    assert.deepEqual(lintVue(root, ["src/legacy"]), {
        status: 1,
        stderr: "",
        stdout: "src/legacy/Kept.vue:2:6: No v-html. [Warning/sfc/no-v-html]\nFound 1 problem in 2 .vue files.\n",
    });
    assert.deepEqual(lintVue(root, ["src/Clean.vue", "src/legacy/Old.vue"]), {
        status: 0,
        stderr: "",
        stdout: "Found 0 problems in 2 .vue files.\n",
    });
    assert.deepEqual(lintVue(root, ["src/Clean.vue"]).stdout, "Found 0 problems in 1 .vue file.\n");
    // An ignored file is not linted, even when it is named.
    assert.deepEqual(lintVue(root, ["generated"]).stdout, "Found 0 problems in 0 .vue files.\n");
});

test("the json format is ESLint's report", t => {
    const root = project(t);
    const run = lintVue(root, ["--format", "json", "src/App.vue", "src/Clean.vue", "src/legacy"]);

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
    assert.deepEqual(lintVue(root, ["-f", "json", "src/Clean.vue"]), { status: 0, stderr: "", stdout: "[]\n" });
});

test("--fix writes the fixed files and reports what is left", t => {
    const root = project(t);

    assert.deepEqual(lintVue(root, ["--fix", "src/App.vue", "src/Clean.vue"]), {
        status: 1,
        stderr: "",
        stdout: "src/App.vue:6:6: No v-html. [Warning/sfc/no-v-html]\nFound 1 problem in 2 .vue files.\n",
    });
    assert.equal(fs.readFileSync(path.join(root, "src/App.vue"), "utf8"), APP.replaceAll("old()", "renamed()"));
    assert.equal(fs.readFileSync(path.join(root, "src/Clean.vue"), "utf8"), CLEAN);
    assert.equal(fs.readFileSync(path.join(root, "src/not-vue.ts"), "utf8"), "old()\n");
});

test("a syntax error of a script is reported without a rule", t => {
    const root = project(t, { "src/Broken.vue": '<script setup lang="ts">\nconst = 1\n</script>\n' });
    const run = lintVue(root, ["src/Broken.vue"]);

    assert.equal(run.status, 1);
    assert.match(run.stdout, /^src\/Broken\.vue:2:\d+: Parsing error: .* \[Error\/vue\]\nFound 1 problem in 1 \.vue file\.\n$/u);
});

test("--config names a configuration elsewhere, and --root the directory it applies to", t => {
    const root = project(t, {
        "config/elsewhere.json": {
            jsPlugins: [{ name: "renamed", specifier: "./plugins/sfc.mjs" }],
            overrides: [{ files: ["src/*.vue"], rules: { "renamed/no-v-html": 2 } }],
        },
    });

    // Without --root, the globs and the plugin path are relative to config/: the plugin is not there.
    const relative = lintVue(root, ["--config", "config/elsewhere.json"]);
    assert.equal(relative.status, 2);
    assert.match(relative.stderr, /^tsrs-lint-vue: Cannot load the plugin \.\/plugins\/sfc\.mjs \(file:.*\/config\/plugins\/sfc\.mjs\)/u);
    assert.deepEqual(lintVue(root, ["-c", "config/elsewhere.json", "--root", "."]), {
        status: 1,
        stderr: "",
        stdout: "src/App.vue:6:6: No v-html. [Error/renamed/no-v-html]\nFound 1 problem in 5 .vue files.\n",
    });
});

test("a lint that cannot run exits with 2 and says why", t => {
    const root = project(t, {
        "bad-rule.json": { jsPlugins: ["./plugins/sfc.mjs"], rules: { "sfc/no-such-rule": 2 } },
        "bad-severity.json": { jsPlugins: ["./plugins/sfc.mjs"], rules: { "sfc/no-old": "loud" } },
    });
    const failure = (args: readonly string[], cwd = root): string => {
        const run = lintVue(cwd, args);
        assert.deepEqual({ status: run.status, stdout: run.stdout }, { status: 2, stdout: "" }, run.stderr);
        return run.stderr;
    };

    assert.match(failure(["--unknown"]), /^tsrs-lint-vue: Unknown option '--unknown'.*\nRun tsrs-lint-vue --help for the usage\.\n$/su);
    assert.equal(failure(["--format", "stylish"]), "tsrs-lint-vue: --format is unix or json, not stylish\nRun tsrs-lint-vue --help for the usage.\n");
    assert.match(failure(["--config", "missing.json"]), /^tsrs-lint-vue: Cannot read the configuration .*missing\.json: ENOENT/u);
    assert.equal(failure(["--config", "bad-rule.json"]), "tsrs-lint-vue: No plugin has a rule named sfc/no-such-rule\n");
    assert.match(failure(["--config", "bad-severity.json"]), /^tsrs-lint-vue: The severity of sfc\/no-old is not one of/u);

    // A project without the parsers: the error names the package to install.
    const bare = project(t, {}, []);
    assert.match(
        failure([], bare),
        /^tsrs-lint-vue: Cannot find the package vue-eslint-parser from .*: add vue-eslint-parser to the project's dependencies\.\n$/u,
    );
    const half = project(t, {}, ["vue-eslint-parser"]);
    assert.match(failure([], half), /^tsrs-lint-vue: Cannot find the package @typescript-eslint\/parser from /u);

    // Not a git repository: the files cannot be listed.
    fs.rmSync(path.join(bare, ".git"), { recursive: true });
    assert.match(failure([], bare), /^tsrs-lint-vue: Cannot list the files of .* with git ls-files: /u);
});

test("a rule that throws fails the lint with the rule, the file and the stack", t => {
    const root = project(t, {
        ".oxlintrc.json": { jsPlugins: ["./plugins/throws.mjs"], rules: { "throws/always": 2 } },
        "plugins/throws.mjs": 'export default { meta: { name: "throws", sfc: true }, rules: { always: { create() { throw new TypeError("boom"); } } } };\n',
    });
    const run = lintVue(root, ["src/Clean.vue"]);

    assert.equal(run.status, 2);
    assert.equal(run.stdout, "");
    assert.match(run.stderr, /^tsrs-lint-vue: Error: throws\/always on .*\/src\/Clean\.vue: boom\n/u);
    assert.match(run.stderr, /\[cause\]: TypeError: boom\n\s+at .*create.*plugins\/throws\.mjs/u);
});

test("--help prints the usage, with the Node.js version the command needs", t => {
    const run = lintVue(project(t), ["--help"]);

    assert.equal(run.status, 0);
    assert.equal(run.stderr, "");
    assert.match(run.stdout, /^Usage: tsrs-lint-vue \[options\] \[paths\.\.\.\]\n/u);
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

test("the type-aware rules run in tsrs-lint through the content mapper, and report in the <script> blocks", { skip: process.env["TSRS_BINARY"] ? false : "needs the tsrs binary of TSRS_BINARY" }, t => {
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
    const run = lintVue(root, ["--project", "check/tsconfig.json", "src/Typed.vue", "src/Clean.vue"]);

    // `{{ load() }}` floats a promise too, and tsrs-lint reports it: a template expression is type-checked, not
    // linted. The column of the second report is in characters: tsrs-lint counts the `é` before it as two bytes.
    assert.equal(run.stderr, "");
    assert.deepEqual(run.stdout.split("\n").map(line => line.replace(/: .* \[/u, " [")), [
        "src/Typed.vue:5:1 [Error/typescript-eslint(no-floating-promises)]",
        "src/Typed.vue:6:27 [Error/typescript-eslint(no-floating-promises)]",
        "src/Typed.vue:7:1 [Warning/typescript-eslint(await-thenable)]",
        "Found 3 problems in 2 .vue files.",
        "",
    ]);
    assert.match(run.stdout, /^src\/Typed\.vue:5:1: Promises must be awaited/u);
    assert.equal(run.status, 1);

    // The tsconfig with the content mapper is not the one nearest to the file: without --project, tsrs-lint does not
    // see the floating promises.
    assert.doesNotMatch(lintVue(root, ["src/Typed.vue"]).stdout, /no-floating-promises/u);
});
