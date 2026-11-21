/**
 * `tsrs-lint-vue [--fix] [--format unix|json] [--config <file>] [--root <dir>] [--project <tsconfig>] [paths]`:
 * lints what Oxlint cannot see of a .vue file. Oxlint lints its `<script>`
 * blocks, as it lints a .ts file, except for two things it has no access to:
 *
 * - the `<template>` and the custom blocks: the rules of the JS plugins of
 *   the configuration that say they read a whole .vue file (`meta.sfc`) run
 *   on vue-eslint-parser's tree, in our own rule host (./host/host.ts);
 * - the types: the type-aware rules that the configuration gives the file
 *   run in `tsrs-lint`, through the content mapper of the file's tsconfig, so
 *   a rule sees the types that the type check sees. They report in the
 *   `<script>` blocks: a template expression is type-checked, not linted
 *   (`@click="save"` with an async `save` is the Vue idiom, and
 *   `no-misused-promises` would ban it).
 *
 * Everything is configured in the Oxlint configuration file, and nothing
 * here is about one project: docs/LINT_VUE.md has the contract.
 *
 * `--fix` applies the fixes of the hosted rules.
 */
import { execFileSync } from "node:child_process";
import {
    existsSync,
    readFileSync,
    writeFileSync,
} from "node:fs";
import {
    join,
    relative,
    resolve,
} from "node:path";
import {
    inspect,
    parseArgs,
} from "node:util";
import {
    verify,
    verifyAndFix,
} from "./host/host.ts";
import type { LintConfig } from "./host/host.ts";
import { isRecord } from "./host/nodes.ts";
import type { LintMessage } from "./host/types.ts";
import {
    isIgnored,
    isOff,
    optionsOf,
    readOxlintConfig,
    rulesFor,
    severityOf,
} from "./oxlintConfig.ts";
import type {
    OxlintConfig,
    RuleEntry,
} from "./oxlintConfig.ts";
import {
    hostedRules,
    loadPlugins,
} from "./plugins.ts";
import type { HostedRules } from "./plugins.ts";
import {
    ConfigurationError,
    requireFromProject,
} from "./project.ts";
import {
    implementedRules,
    runTsrsLint,
} from "./tsrsLint.ts";
import type {
    TypeAwareGroup,
    TypeAwareRule,
} from "./tsrsLint.ts";

const OXLINT_CONFIG = ".oxlintrc.json";
const TYPESCRIPT_RULE = /^(?:typescript|@typescript-eslint)\/(?<name>.+)$/u;
const WARNING = 1;
const ERROR = 2;

const USAGE = `Usage: tsrs-lint-vue [options] [paths...]

Lints what Oxlint cannot see of the .vue files under the paths (default: the current directory, which is the root
of the project): the <template> and the custom blocks, with the rules of the JS plugins of the configuration whose
meta.sfc is true, and the types of the <script> blocks, with the type-aware rules that tsrs-lint implements.

Options:
  --fix                     apply the fixes of the plugin rules
  -f, --format <format>     unix (the default) or json (ESLint's JSON report)
  -c, --config <file>       the Oxlint configuration (default: ${OXLINT_CONFIG} in the current directory)
  --root <dir>              the directory that the configuration applies to: its globs and its relative plugin
                            paths are resolved against it (default: the directory of the configuration file)
  -p, --project <tsconfig>  the solution that tsrs-lint lints the files in (tsrs-lint headless --project)
  -h, --help                print this message

It runs on Node.js 22.18 or later: a plugin is an ES module, which may be written in TypeScript. The project
provides vue-eslint-parser and @typescript-eslint/parser (plugin rules) and @vue/compiler-sfc (type-aware rules).

Exit status: 0 without a problem, 1 with problems (a warning is one), 2 when the lint could not run (usage,
configuration, plugin or tsrs-lint error).
`;

interface Problem {
    column: number;
    file: string;
    line: number;
    message: string;
    rule: string;
    severity: number;
}

interface FileRules {
    /** Every rule entry of the file, by the name it has in the configuration. */
    all: Map<string, RuleEntry>;
    /** The entries of the rules that the rule host runs. */
    hosted: Record<string, RuleEntry>;
    /** The severity of each type-aware rule, by the name `tsrs-lint` reports it with. */
    severities: Map<string, number>;
    typeAware: TypeAwareRule[];
}

/** A .vue file as both passes need it. */
interface VueFile {
    /** Relative to the root of the project. */
    file: string;
    rules: FileRules;
    source: string;
}

/** A run of the lint: the project and what the command line and the configuration say. */
interface Lint {
    config: OxlintConfig;
    fix: boolean;
    hosted: HostedRules;
    /** The tsconfig of `--project`. */
    project: string | undefined;
    /** The root of the project: the current directory. */
    root: string;
    /** The names of the rules that `tsrs-lint` implements. */
    typeAwareNames: ReadonlySet<string>;
}

/** `file` (relative to the root of the project) as the globs of the configuration name it. */
const configPath = ({ config, root }: Pick<Lint, "config" | "root">, file: string): string => relative(config.directory, join(root, file));

/** The .vue files under `paths` that the configuration does not ignore, relative to the root. */
function vueFiles(lint: Pick<Lint, "config" | "root">, paths: readonly string[]): string[] {
    let listed: string;
    try {
        listed = execFileSync(
            "git",
            ["ls-files", "-co", "--exclude-standard", "-z", "--", ...paths],
            { cwd: lint.root, encoding: "utf8", maxBuffer: Number.MAX_SAFE_INTEGER, stdio: ["ignore", "pipe", "pipe"] },
        );
    }
    catch (error) {
        const stderr = isRecord(error) && typeof error["stderr"] === "string" ? error["stderr"].trim() : "";
        throw new ConfigurationError(
            `Cannot list the files of ${lint.root} with git ls-files: ${stderr || (error instanceof Error ? error.message : String(error))}`,
            { cause: error },
        );
    }

    return listed
        .split("\0")
        .filter(file =>
            file.endsWith(".vue")
            && !isIgnored(lint.config, configPath(lint, file))
            // A file that is deleted and still in the index.
            && existsSync(join(lint.root, file))
        )
        .sort();
}

/** Oxlint's `allow`, `warn` and `deny` as the numbers ESLint takes too. */
function severityNumber(entry: RuleEntry): number {
    if (isOff(entry)) {
        return 0;
    }
    const severity = severityOf(entry);

    return severity === WARNING || severity === "warn" ? WARNING : ERROR;
}

/** Whether a layer of the configuration names a rule of typescript-eslint: `tsrs-lint` says which are its own. */
const namesTypeScriptRules = (config: OxlintConfig): boolean =>
    [config.rules, ...config.overrides.map(override => override.rules)].some(rules => Object.keys(rules).some(rule => TYPESCRIPT_RULE.test(rule)));

function rulesOf(lint: Lint, file: string): FileRules {
    const all = rulesFor(lint.config, configPath(lint, file));
    const typeAware: TypeAwareRule[] = [];
    const severities = new Map<string, number>();
    for (const [rule, entry] of all) {
        const name = TYPESCRIPT_RULE.exec(rule)?.groups?.["name"];
        if (name !== undefined && lint.typeAwareNames.has(name) && !isOff(entry)) {
            const [options] = optionsOf(entry);
            typeAware.push(options === undefined ? { name } : { name, options });
            severities.set(name, severityNumber(entry));
        }
    }

    return {
        all,
        hosted: lint.hosted.entriesOf(all),
        severities,
        typeAware: typeAware.sort((a, b) => a.name.localeCompare(b.name)),
    };
}

/** Lints a file with the hosted rules. With `fix`, applies the fixes and updates `vue` to the fixed file. */
function lintHosted(lint: Lint, vue: VueFile): Problem[] {
    if (Object.values(vue.rules.hosted).every(isOff)) {
        return [];
    }
    const path = join(lint.root, vue.file);
    const options: LintConfig = {
        cwd: lint.root,
        fileRules: vue.rules.all,
        filename: path,
        ruleOf: lint.hosted.ruleOf,
        rules: vue.rules.hosted,
        settings: lint.config.settings,
    };
    const run = (): LintMessage[] => {
        if (!lint.fix) {
            return verify(vue.source, options);
        }
        const fixed = verifyAndFix(vue.source, options);
        if (fixed.fixed) {
            writeFileSync(path, fixed.output);
            // The messages, and then the type-aware diagnostics, are about the
            // fixed file.
            vue.source = fixed.output;
        }

        return fixed.messages;
    };

    return run().map(message => ({
        column: message.column,
        file: vue.file,
        line: message.line,
        message: message.message,
        // A syntax error of a `<script>` block has no rule.
        rule: message.ruleId ?? "vue",
        severity: message.severity,
    }));
}

interface Block {
    end: number;
    start: number;
}

/** The `<script>` blocks of a .vue source, as UTF-16 offsets. */
type ScriptBlocks = (source: string, file: string) => Block[];

function offsetOf(block: Record<string, unknown>, edge: "end" | "start"): number | undefined {
    const { loc } = block;
    const position = isRecord(loc) ? loc[edge] : undefined;
    const offset = isRecord(position) ? position["offset"] : undefined;

    return typeof offset === "number" ? offset : undefined;
}

/** How the project's @vue/compiler-sfc splits a .vue file: it is loaded untyped. */
function scriptBlocksOf(root: string): ScriptBlocks {
    const compiler = requireFromProject(
        root,
        "@vue/compiler-sfc",
        "finds the <script> blocks, which the type-aware rules report in,",
    );
    const parse = isRecord(compiler) ? compiler["parse"] : undefined;
    if (typeof parse !== "function") {
        throw new TypeError("@vue/compiler-sfc is not the compiler that the Vue lint expects");
    }

    return (source, file) => {
        const parsed: unknown = parse(source, { filename: file });
        const descriptor = isRecord(parsed) && isRecord(parsed["descriptor"]) ? parsed["descriptor"] : {};

        return [descriptor["script"], descriptor["scriptSetup"]].flatMap(block => {
            const start = isRecord(block) ? offsetOf(block, "start") : undefined;
            const end = isRecord(block) ? offsetOf(block, "end") : undefined;

            return start === undefined || end === undefined ? [] : [{ end, start }];
        });
    };
}

/** The 1-based line and column of the UTF-16 offset `offset` of `source`. */
function locate(source: string, offset: number): { column: number; line: number; } {
    const before = source.slice(0, offset);
    const lineStart = before.lastIndexOf("\n") + 1;

    return {
        column: offset - lineStart + 1,
        line: before.split("\n").length,
    };
}

/** Lints the scripts of the files with the type-aware rules of each. */
function lintTypes(lint: Lint, files: ReadonlyMap<string, VueFile>): Problem[] {
    const groups = new Map<string, TypeAwareGroup>();
    for (const [file, { rules }] of files) {
        if (rules.typeAware.length > 0) {
            const key = JSON.stringify(rules.typeAware);
            const group = groups.get(key) ?? { files: [], rules: rules.typeAware };
            group.files.push(file);
            groups.set(key, group);
        }
    }
    if (groups.size === 0) {
        return [];
    }
    const scriptBlocks = scriptBlocksOf(lint.root);

    return runTsrsLint(lint.root, lint.project, [...groups.values()]).flatMap(diagnostic => {
        const file = relative(lint.root, diagnostic.file);
        const vue = files.get(file);
        const severity = vue?.rules.severities.get(diagnostic.rule);
        if (vue === undefined || severity === undefined) {
            // Not a rule's: a tsconfig or mapper error.
            return [
                {
                    column: 1,
                    file,
                    line: 1,
                    message: diagnostic.message,
                    rule: `typescript(${diagnostic.rule})`,
                    severity: ERROR,
                },
            ];
        }
        // tsrs-lint counts in UTF-8 bytes.
        const offset = Buffer.from(vue.source)
            .subarray(0, diagnostic.pos ?? 0)
            .toString("utf8").length;

        return scriptBlocks(vue.source, vue.file).some(block => offset >= block.start && offset < block.end)
            ? [
                {
                    ...locate(vue.source, offset),
                    file,
                    message: diagnostic.message,
                    rule: `typescript-eslint(${diagnostic.rule})`,
                    severity,
                },
            ]
            : [];
    });
}

const textReport = (problems: readonly Problem[], files: number): string =>
    [
        ...problems.map(
            problem => `${problem.file}:${problem.line}:${problem.column}: ${problem.message} [${problem.severity === WARNING ? "Warning" : "Error"}/${problem.rule}]`,
        ),
        `Found ${problems.length} problem${problems.length === 1 ? "" : "s"} in ${files} .vue file${files === 1 ? "" : "s"}.`,
        "",
    ].join("\n");

/** The problems as ESLint's `json` formatter reports them. */
function jsonReport(root: string, problems: readonly Problem[]): unknown[] {
    const byFile = new Map<string, Problem[]>();
    for (const problem of problems) {
        byFile.set(problem.file, [...(byFile.get(problem.file) ?? []), problem]);
    }

    return [...byFile].map(([file, messages]) => ({
        filePath: join(root, file),
        messages: messages.map(problem => ({
            column: problem.column,
            line: problem.line,
            message: problem.message,
            ruleId: problem.rule,
            severity: problem.severity,
        })),
    }));
}

interface CommandLine {
    config: string | undefined;
    fix: boolean;
    format: string;
    help: boolean;
    paths: string[];
    project: string | undefined;
    root: string | undefined;
}

function parseCommandLine(args: readonly string[]): CommandLine {
    try {
        const { positionals, values } = parseArgs({
            allowPositionals: true,
            args: [...args],
            options: {
                config: { short: "c", type: "string" },
                fix: { default: false, type: "boolean" },
                // `json`: ESLint's report format, for the tools that read one.
                format: { default: "unix", short: "f", type: "string" },
                help: { default: false, short: "h", type: "boolean" },
                project: { short: "p", type: "string" },
                root: { type: "string" },
            },
        });
        if (values.format !== "unix" && values.format !== "json") {
            throw new TypeError(`--format is unix or json, not ${values.format}`);
        }

        return {
            config: values.config,
            fix: values.fix,
            format: values.format,
            help: values.help,
            paths: positionals,
            project: values.project,
            root: values.root,
        };
    }
    catch (error) {
        throw new ConfigurationError(
            `${error instanceof Error ? error.message : String(error)}\nRun tsrs-lint-vue --help for the usage.`,
            { cause: error },
        );
    }
}

async function run(args: readonly string[], root: string): Promise<number> {
    const commandLine = parseCommandLine(args);
    if (commandLine.help) {
        process.stdout.write(USAGE);

        return 0;
    }
    const configFile = resolve(root, commandLine.config ?? OXLINT_CONFIG);
    const config = readOxlintConfig(
        configFile,
        commandLine.root === undefined ? undefined : resolve(root, commandLine.root),
    );
    const files = vueFiles(
        { config, root },
        commandLine.paths.length > 0
            ? commandLine.paths.map(path => relative(root, resolve(root, path)) || ".")
            : ["."],
    );
    const lint: Lint = {
        config,
        fix: commandLine.fix,
        hosted: hostedRules(await loadPlugins(config, root)),
        project: commandLine.project,
        root,
        typeAwareNames: files.length > 0 && namesTypeScriptRules(config) ? implementedRules(root) : new Set(),
    };
    const vueByFile = new Map(
        files.map(file => [
            file,
            { file, rules: rulesOf(lint, file), source: readFileSync(join(root, file), "utf8") },
        ]),
    );
    const problems = [
        ...[...vueByFile.values()].flatMap(vue => lintHosted(lint, vue)),
        ...lintTypes(lint, vueByFile),
    ].sort((a, b) => a.file.localeCompare(b.file) || a.line - b.line || a.column - b.column);
    process.stdout.write(
        commandLine.format === "json"
            ? `${JSON.stringify(jsonReport(root, problems))}\n`
            : textReport(problems, files.length),
    );

    // As with Oxlint's `--max-warnings 0`, a warning fails the lint.
    return problems.length > 0 ? 1 : 0;
}

/**
 * Runs the command with `args`, in the project at `root`, and returns its exit status. A lint that could not run says
 * why on stderr: a mistake of the command line, of the configuration or of the project in one line, anything else
 * (a rule or `tsrs-lint` that failed) with where it comes from.
 */
export async function main(args: readonly string[], root: string = process.cwd()): Promise<number> {
    try {
        return await run(args, root);
    }
    catch (error) {
        process.stderr.write(`tsrs-lint-vue: ${error instanceof ConfigurationError ? error.message : inspect(error)}\n`);

        return 2;
    }
}
