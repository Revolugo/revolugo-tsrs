/**
 * `tsrs-lint [--fix] [--format unix|json] [--config <file>] [--root <dir>] [--project <tsconfig>] [--no-oxlint]
 * [paths]`: lints the files of a project with every rule of its Oxlint
 * configuration, and prints one report.
 *
 * - Oxlint runs its own rules and the rules of the JS plugins that the lint
 *   does not run itself (./oxlint.ts), without `--type-aware`.
 * - The rules that `tsrs-lint headless` implements run there, on the script
 *   files and on the .vue files: the rules of typescript-eslint that need the
 *   types, and the rules that a JS plugin leaves to it (`meta.native`), which
 *   are ports of its own rules. A .vue file goes through the content mapper
 *   of its tsconfig, so a rule sees the types that the type check sees. It
 *   gets the rules of typescript-eslint only, and they report in the
 *   `<script>` blocks: a template expression is type-checked, not linted
 *   (`@click="save"` with an async `save` is the Vue idiom, and
 *   `no-misused-promises` would ban it).
 * - The `<template>` and the custom blocks of a .vue file, which Oxlint has
 *   no access to: the rules of the JS plugins that say they read a whole .vue
 *   file (`meta.sfc`) run on vue-eslint-parser's tree, in our own rule host
 *   (./host/host.ts).
 *
 * The three run at the same time. Everything is configured in the Oxlint
 * configuration file, and nothing here is about one project: docs/LINT.md has
 * the contract.
 *
 * `--fix` applies the fixes of Oxlint, then those of the hosted rules, then
 * those of `tsrs-lint` in the script files: one after the other, since each
 * rewrites files that the next one reads.
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
import { runOxlint } from "./oxlint.ts";
import type { OxlintProblem } from "./oxlint.ts";
import {
    hostedRules,
    loadPlugins,
    nativeRuleNames,
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
    NativeDiagnostic,
    NativeFix,
    NativeGroup,
    NativeRule,
} from "./tsrsLint.ts";

const OXLINT_CONFIG = ".oxlintrc.json";
const TYPESCRIPT_RULE = /^(?:typescript|@typescript-eslint)\/(?<name>.+)$/u;
const WARNING = 1;
const ERROR = 2;

/** What Oxlint lints as a script. */
const SCRIPT = /\.(?:[cm]?[jt]s|[jt]sx)$/u;
const VUE = /\.vue$/u;
/** As ESLint: a fix can uncover another one, and a rule whose fix never settles does not hang the lint. */
const MAX_FIX_PASSES = 10;

const USAGE = `Usage: tsrs-lint [options] [paths...]
       tsrs-lint headless [--fix] [--runExternalCode] [--project <tsconfig>]
       tsrs-lint rules

Lints the files under the paths (default: the current directory, which is the root of the project) with the rules of
the Oxlint configuration, and prints one report:

  - Oxlint, from the project, runs its own rules and the rules of the JS plugins that tsrs-lint does not run;
  - in the script files and in the <script> blocks of the .vue files, tsrs-lint runs the rules that it implements
    (tsrs-lint rules): the rules of typescript-eslint that need the types and, in the script files, the rules that a
    JS plugin leaves to tsrs-lint (meta.native);
  - in the <template> and the custom blocks of the .vue files, tsrs-lint runs the rules of the JS plugins whose
    meta.sfc is true.

Options:
  --fix                     apply the fixes of Oxlint, of the plugin rules, and of tsrs-lint in the script files
  -f, --format <format>     unix (the default) or json (ESLint's JSON report)
  -c, --config <file>       the Oxlint configuration (default: ${OXLINT_CONFIG} in the current directory)
  --root <dir>              the directory that the configuration applies to: its globs and its relative plugin
                            paths are resolved against it (default: the directory of the configuration file)
  -p, --project <tsconfig>  the solution that tsrs-lint lints the files in (tsrs-lint headless --project)
  --no-oxlint               leave Oxlint out: lint only with the rules that Oxlint does not run
  -h, --help                print this message

tsrs-lint headless is the backend that Oxlint drives with --type-aware (a payload on stdin, frames on stdout), and
tsrs-lint rules lists the rules it implements.

It runs on Node.js 22.18 or later: a plugin is an ES module, which may be written in TypeScript. The project provides
oxlint and, to lint .vue files, vue-eslint-parser and @typescript-eslint/parser (plugin rules) and @vue/compiler-sfc
(rules of tsrs-lint).

Exit status: 0 without a problem, 1 with problems (a warning is one), 2 when the lint could not run (usage,
configuration, plugin, Oxlint or tsrs-lint error).
`;

interface Problem {
    column: number;
    file: string;
    line: number;
    message: string;
    rule: string;
    severity: number;
}

/** How a rule of `tsrs-lint` is reported: the severity the configuration gives it, and the name it has there. */
interface Reported {
    rule: string;
    severity: number;
}

interface FileRules {
    /** Every rule entry of the file, by the name it has in the configuration. */
    all: Map<string, RuleEntry>;
    /** The entries of the rules that the rule host runs. */
    hosted: Record<string, RuleEntry>;
    /** The rules that `tsrs-lint` runs. */
    native: NativeRule[];
    /** How each rule of `native` is reported, by the name `tsrs-lint` has for it. */
    reported: Map<string, Reported>;
}

/** A file to lint. */
interface LintedFile {
    /** Relative to the root of the project. */
    file: string;
    rules: FileRules;
    /** The text of a .vue file, which both passes read. A script file is read when something is reported in it. */
    source: string | undefined;
    vue: boolean;
}

/** A run of the lint: the project and what the command line and the configuration say. */
interface Lint {
    config: OxlintConfig;
    fix: boolean;
    hosted: HostedRules;
    /** The names of the rules that `tsrs-lint` implements. */
    implemented: ReadonlySet<string>;
    /** The name `tsrs-lint` has for each rule that a plugin leaves to it, by its name in the configuration. */
    native: ReadonlyMap<string, string>;
    /** The tsconfig of `--project`. */
    project: string | undefined;
    /** The root of the project: the current directory. */
    root: string;
    /** Aborted when the lint ends: a `tsrs-lint` that still runs then, after a rule threw, is stopped. */
    signal: AbortSignal;
}

/** `file` (relative to the root of the project) as the globs of the configuration name it. */
const configPath = ({ config, root }: Pick<Lint, "config" | "root">, file: string): string => relative(config.directory, join(root, file));

/** The files that `git ls-files` lists under `paths` with `flags`, relative to the root. */
function gitFiles(root: string, flags: string, paths: readonly string[]): string[] {
    try {
        return execFileSync(
            "git",
            ["ls-files", flags, "--exclude-standard", "-z", "--", ...paths],
            { cwd: root, encoding: "utf8", maxBuffer: Number.MAX_SAFE_INTEGER, stdio: ["ignore", "pipe", "pipe"] },
        ).split("\0");
    }
    catch (error) {
        const stderr = isRecord(error) && typeof error["stderr"] === "string" ? error["stderr"].trim() : "";
        throw new ConfigurationError(
            `Cannot list the files of ${root} with git ls-files: ${stderr || (error instanceof Error ? error.message : String(error))}`,
            { cause: error },
        );
    }
}

/**
 * The script and .vue files under `paths` that Oxlint lints too, relative to the root: those that neither git nor
 * the configuration ignores.
 */
function lintedFiles(lint: Pick<Lint, "config" | "root">, paths: readonly string[]): string[] {
    // A file that .gitignore names is ignored even when it is tracked, as in Oxlint.
    const ignored = new Set(gitFiles(lint.root, "-ci", paths));

    return [...new Set(gitFiles(lint.root, "-co", paths))]
        .filter(file =>
            (SCRIPT.test(file) || VUE.test(file))
            && !ignored.has(file)
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

/** Whether a layer of the configuration names a rule that `tsrs-lint` may run: it says which ones it implements. */
const namesNativeRules = (config: OxlintConfig, native: ReadonlyMap<string, string>): boolean =>
    [config.rules, ...config.overrides.map(override => override.rules)].some(rules =>
        Object.keys(rules).some(rule => TYPESCRIPT_RULE.test(rule) || native.has(rule))
    );

function rulesOf(lint: Lint, file: string): FileRules {
    const all = rulesFor(lint.config, configPath(lint, file));
    const vue = VUE.test(file);
    const native: NativeRule[] = [];
    const reported = new Map<string, Reported>();
    for (const [rule, entry] of all) {
        if (isOff(entry)) {
            continue;
        }
        const typed = TYPESCRIPT_RULE.exec(rule)?.groups?.["name"];
        if (typed !== undefined) {
            // A rule of typescript-eslint that tsrs-lint does not have is Oxlint's, not ours.
            if (lint.implemented.has(typed)) {
                // typescript-eslint gives a rule one options object.
                const [options] = optionsOf(entry);
                native.push(options === undefined ? { name: typed } : { name: typed, options });
                reported.set(typed, { rule: `typescript-eslint(${typed})`, severity: severityNumber(entry) });
            }
            continue;
        }
        const ported = vue ? undefined : lint.native.get(rule);
        if (ported !== undefined) {
            if (!lint.implemented.has(ported)) {
                throw new ConfigurationError(
                    `The plugin of ${rule} leaves the rule to tsrs-lint (meta.native), and this tsrs-lint has no rule ${ported}: \`tsrs-lint rules\` lists those it has`,
                );
            }
            const options = optionsOf(entry);
            native.push(options.length === 0 ? { name: ported } : { name: ported, options });
            // As Oxlint names the rule: it reported it before tsrs-lint had a port of it.
            const slash = rule.lastIndexOf("/");
            reported.set(ported, { rule: `${rule.slice(0, slash)}(${rule.slice(slash + 1)})`, severity: severityNumber(entry) });
        }
    }

    return {
        all,
        hosted: vue ? lint.hosted.entriesOf(all) : {},
        native: native.sort((a, b) => a.name.localeCompare(b.name)),
        reported,
    };
}

/** Lints a file with the hosted rules. With `fix`, applies the fixes and updates `vue` to the fixed file. */
function lintHosted(lint: Lint, vue: LintedFile): Problem[] {
    const { source } = vue;
    if (source === undefined || Object.values(vue.rules.hosted).every(isOff)) {
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
            return verify(source, options);
        }
        const fixed = verifyAndFix(source, options);
        if (fixed.fixed) {
            writeFileSync(path, fixed.output);
            // The messages, and then what tsrs-lint reports, are about the fixed
            // file.
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
        "finds the <script> blocks, which the rules of tsrs-lint report in,",
    );
    const parse = isRecord(compiler) ? compiler["parse"] : undefined;
    if (typeof parse !== "function") {
        throw new TypeError("@vue/compiler-sfc is not the compiler that the lint expects");
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

/** The files to lint with `tsrs-lint`, grouped by the rules they have. */
function nativeGroups(files: Iterable<LintedFile>): NativeGroup[] {
    const groups = new Map<string, NativeGroup>();
    for (const { file, rules } of files) {
        if (rules.native.length > 0) {
            const key = JSON.stringify(rules.native);
            const group = groups.get(key) ?? { files: [], rules: rules.native };
            group.files.push(file);
            groups.set(key, group);
        }
    }

    return [...groups.values()];
}

/**
 * Applies to the script file at `path` the fixes of what `tsrs-lint` reports in it, the first ones first. A fix
 * whose range touches that of an earlier one waits for the next pass. Returns whether the file changed.
 */
function applyFixes(path: string, diagnostics: readonly NativeDiagnostic[]): boolean {
    const source = readFileSync(path);
    const applied: NativeFix[] = [];
    for (const { fixes } of diagnostics) {
        // The fixes of one problem go together.
        if (fixes.length > 0 && fixes.every(fix => applied.every(other => fix.end < other.pos || fix.pos > other.end))) {
            applied.push(...fixes);
        }
    }
    if (applied.length === 0) {
        return false;
    }
    const parts: Buffer[] = [];
    let offset = 0;
    for (const fix of applied.sort((a, b) => a.pos - b.pos || a.end - b.end)) {
        parts.push(source.subarray(offset, fix.pos), Buffer.from(fix.text));
        offset = fix.end;
    }
    writeFileSync(path, Buffer.concat([...parts, source.subarray(offset)]));

    return true;
}

/**
 * What `tsrs-lint` reports in `files`, by file (relative to the root). With `--fix`, the fixes of the script files
 * are applied, and the files that changed are linted again, until nothing changes: the result is about the files
 * as they are left.
 */
async function runNative(lint: Lint, files: ReadonlyMap<string, LintedFile>): Promise<Map<string, NativeDiagnostic[]>> {
    const byFile = new Map<string, NativeDiagnostic[]>();
    let pending = [...files.values()];
    for (let pass = 0; pending.length > 0 && nativeGroups(pending).length > 0; pass++) {
        // After the last pass that fixes, one more reports what is left.
        const fix = lint.fix && pass < MAX_FIX_PASSES && pending.some(file => !file.vue);
        for (const { file } of pending) {
            byFile.delete(file);
        }
        const diagnostics = await runTsrsLint(
            { fix, project: lint.project, root: lint.root, signal: lint.signal },
            nativeGroups(pending),
        );
        for (const diagnostic of diagnostics) {
            const file = relative(lint.root, diagnostic.file);
            byFile.set(file, [...(byFile.get(file) ?? []), diagnostic]);
        }
        pending = fix
            ? pending.filter(({ file, vue }) => !vue && applyFixes(join(lint.root, file), byFile.get(file) ?? []))
            : [];
    }

    return byFile;
}

/** Lints the files with the rules of `tsrs-lint` that each has. */
async function lintNative(lint: Lint, files: ReadonlyMap<string, LintedFile>): Promise<Problem[]> {
    const reports = await runNative(lint, files);
    if (reports.size === 0) {
        return [];
    }
    let blocksOf: ScriptBlocks | undefined;

    return [...reports].flatMap(([file, diagnostics]) => {
        const linted = files.get(file);
        /** The file as `tsrs-lint` counts in it, in UTF-8 bytes, and as a line and a column count in it. */
        let text: { bytes: Buffer; source: string; } | undefined;

        return diagnostics.flatMap(diagnostic => {
            const reported = linted?.rules.reported.get(diagnostic.rule);
            if (linted === undefined || reported === undefined) {
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
            if (text === undefined) {
                const bytes = linted.source === undefined ? readFileSync(join(lint.root, file)) : Buffer.from(linted.source);
                text = { bytes, source: bytes.toString("utf8") };
            }
            const offset = text.bytes.subarray(0, diagnostic.pos ?? 0).toString("utf8").length;
            if (linted.vue) {
                blocksOf ??= scriptBlocksOf(lint.root);
                if (!blocksOf(text.source, file).some(block => offset >= block.start && offset < block.end)) {
                    return [];
                }
            }

            return [{ ...locate(text.source, offset), file, message: diagnostic.message, ...reported }];
        });
    });
}

const textReport = (problems: readonly Problem[], files: number): string =>
    [
        ...problems.map(
            problem => `${problem.file}:${problem.line}:${problem.column}: ${problem.message} [${problem.severity === WARNING ? "Warning" : "Error"}/${problem.rule}]`,
        ),
        `Found ${problems.length} problem${problems.length === 1 ? "" : "s"} in ${files} file${files === 1 ? "" : "s"}.`,
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
    oxlint: boolean;
    paths: string[];
    project: string | undefined;
    root: string | undefined;
}

function parseCommandLine(args: readonly string[]): CommandLine {
    try {
        const { positionals, values } = parseArgs({
            allowNegative: true,
            allowPositionals: true,
            args: [...args],
            options: {
                config: { short: "c", type: "string" },
                fix: { default: false, type: "boolean" },
                // `json`: ESLint's report format, for the tools that read one.
                format: { default: "unix", short: "f", type: "string" },
                help: { default: false, short: "h", type: "boolean" },
                oxlint: { default: true, type: "boolean" },
                project: { short: "p", type: "string" },
                root: { type: "string" },
            },
        });
        if (values.format !== "unix" && values.format !== "json") {
            throw new TypeError(`--format is unix or json, not ${values.format}`);
        }
        if (values.root !== undefined && values.oxlint) {
            // Oxlint resolves them against the directory of the configuration, and has no option to say otherwise.
            throw new TypeError("--root needs --no-oxlint: Oxlint resolves the globs of the configuration against its directory");
        }

        return {
            config: values.config,
            fix: values.fix,
            format: values.format,
            help: values.help,
            oxlint: values.oxlint,
            paths: positionals,
            project: values.project,
            root: values.root,
        };
    }
    catch (error) {
        throw new ConfigurationError(
            `${error instanceof Error ? error.message : String(error)}\nRun tsrs-lint --help for the usage.`,
            { cause: error },
        );
    }
}

async function run(args: readonly string[], root: string, signal: AbortSignal): Promise<number> {
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
    const paths = commandLine.paths.length > 0
        ? commandLine.paths.map(path => relative(root, resolve(root, path)) || ".")
        : ["."];
    const files = lintedFiles({ config, root }, paths);
    const plugins = await loadPlugins(config, root);
    const native = nativeRuleNames(plugins);
    const lint: Lint = {
        config,
        fix: commandLine.fix,
        hosted: hostedRules(plugins),
        implemented: files.length > 0 && namesNativeRules(config, native) ? implementedRules(root) : new Set(),
        native,
        project: commandLine.project,
        root,
        signal,
    };
    const oxlint = (): Promise<OxlintProblem[]> =>
        commandLine.oxlint
            ? runOxlint({ config: configFile, fix: lint.fix, paths, root, signal })
            : Promise.resolve([]);
    // With --fix, Oxlint goes first: the others read the files that it fixes.
    const fixedByOxlint = lint.fix ? await oxlint() : undefined;
    const linted = new Map<string, LintedFile>(
        files.map(file => {
            const vue = VUE.test(file);

            return [file, { file, rules: rulesOf(lint, file), source: vue ? readFileSync(join(root, file), "utf8") : undefined, vue }];
        }),
    );
    // Oxlint and tsrs-lint run while the rule host does. With --fix, tsrs-lint waits: the rule host rewrites the .vue
    // files, which tsrs-lint reads, a file that it lints as much as a file that another one imports.
    const early = lint.fix ? undefined : Promise.all([oxlint(), lintNative(lint, linted)]);
    // A failure is reported where the result is awaited, after the rule host.
    early?.catch(() => undefined);
    const hosted = [...linted.values()].flatMap(file => lintHosted(lint, file));
    const [byOxlint, byTsrsLint] = early === undefined ? [fixedByOxlint ?? [], await lintNative(lint, linted)] : await early;
    const problems = [
        ...byOxlint,
        ...hosted,
        ...byTsrsLint,
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
    const ended = new AbortController();
    try {
        return await run(args, root, ended.signal);
    }
    catch (error) {
        process.stderr.write(`tsrs-lint: ${error instanceof ConfigurationError ? error.message : inspect(error)}\n`);

        return 2;
    }
    finally {
        ended.abort();
    }
}
