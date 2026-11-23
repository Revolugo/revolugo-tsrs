// What the tsrs-lint tests share: the project that provides the parsers (npm/lint-vue-project), temporary
// projects to lint, and the command run on its sources.
import {
    execFileSync,
    spawnSync,
} from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import type test from "node:test";
import { fileURLToPath } from "node:url";

/** A project that has what the command takes from one: vue-eslint-parser, @typescript-eslint/parser, @vue/compiler-sfc. */
export const PROJECT: string = fileURLToPath(new URL("../../../lint-vue-project", import.meta.url));

const BIN = fileURLToPath(new URL("../../bin/tsrs-lint", import.meta.url));
const PROVIDED = ["vue-eslint-parser", "@typescript-eslint/parser", "@vue/compiler-sfc", "oxlint"];

/** A plugin whose rules the rule host runs: `no-v-html` reports, `no-old` reports and fixes. */
export const SFC_PLUGIN = `
const noVHtml = {
    create: context =>
        context.sourceCode.parserServices.defineTemplateBodyVisitor({
            "VAttribute[directive=true][key.name.name='html']"(node) {
                context.report({ message: "No v-html.", node });
            },
        }),
};
const noOld = {
    meta: { fixable: "code", messages: { old: "Rename {{ name }}." } },
    create(context) {
        const report = node => {
            if (node.name === "old") {
                context.report({
                    data: { name: node.name },
                    fix: fixer => fixer.replaceText(node, "renamed"),
                    messageId: "old",
                    node,
                });
            }
        };
        return context.sourceCode.parserServices.defineTemplateBodyVisitor({ Identifier: report }, { Identifier: report });
    },
};
export default { meta: { name: "sfc", sfc: true }, rules: { "no-old": noOld, "no-v-html": noVHtml } };
`;

/**
 * Creates a git repository with `files` (by path, an object is written as JSON), and with the packages of `provided`
 * in its `node_modules`. It is removed after the test.
 */
export function createProject(
    t: test.TestContext,
    files: Readonly<Record<string, unknown>>,
    provided: readonly string[] = PROVIDED,
): string {
    // The command lists the files of the directory it runs in, which is not a symbolic link there.
    const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tsrs-lint-")));
    t.after(() => fs.rmSync(root, { recursive: true, force: true }));
    for (const name of provided) {
        const link = path.join(root, "node_modules", name);
        fs.mkdirSync(path.dirname(link), { recursive: true });
        fs.symlinkSync(path.join(PROJECT, "node_modules", name), link, "dir");
    }
    for (const [file, content] of Object.entries({ ".gitignore": "node_modules\n", ...files })) {
        const to = path.join(root, file);
        fs.mkdirSync(path.dirname(to), { recursive: true });
        fs.writeFileSync(to, typeof content === "string" ? content : JSON.stringify(content, undefined, 4));
    }
    execFileSync("git", ["init", "--quiet"], { cwd: root });
    return root;
}

export interface Run {
    status: number | null;
    stderr: string;
    stdout: string;
}

/** Runs `tsrs-lint` from its sources, in `cwd`, without Oxlint: most tests are about the rules that it does not run. */
export const lint = (cwd: string, args: readonly string[] = [], env: NodeJS.ProcessEnv = process.env): Run =>
    lintWithOxlint(cwd, ["--no-oxlint", ...args], env);

/** Runs `tsrs-lint` from its sources, in `cwd`. */
export function lintWithOxlint(cwd: string, args: readonly string[] = [], env: NodeJS.ProcessEnv = process.env): Run {
    const run = spawnSync(process.execPath, ["--conditions", "@typescript/source", BIN, ...args], {
        cwd,
        encoding: "utf8",
        env,
    });
    return { status: run.status, stderr: run.stderr, stdout: run.stdout };
}
