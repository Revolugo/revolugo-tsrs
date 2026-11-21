/**
 * The rule host of the Vue lint (../cli.ts): what ESLint's `Linter` does for
 * a rule. It parses a .vue file, creates each rule with a rule context, walks
 * the script tree for the listeners of the rules, and returns their reports,
 * fixed or not.
 *
 * A rule reads the `<template>` through vue-eslint-parser's
 * `parserServices.defineTemplateBodyVisitor`: its template listeners are
 * esquery selectors, which the parser dispatches when the walk leaves the
 * Program. The script listeners are node types: the host has no selector
 * engine (./walk.ts), and rejects a rule that asks for one. A custom block
 * (`<i18n>`) is linted through ./customBlocks.ts.
 *
 * No comment disables a rule: there is no `eslint-disable` directive here.
 */
import { ConfigurationError } from "../project.ts";
import { defineCustomBlocksVisitor } from "./customBlocks.ts";
import {
    applyFixes,
    fixOf,
} from "./fixes.ts";
import type { Fixed } from "./fixes.ts";
import { parseVue } from "./parser.ts";
import { createSourceCode } from "./sourceCode.ts";
import type {
    HostedRule,
    LintMessage,
    ReportDescriptor,
    RuleContext,
    Severity,
    SourceCode,
    SourceLocation,
} from "./types.ts";
import {
    addListeners,
    walk,
} from "./walk.ts";
import type { Listeners } from "./walk.ts";

export interface LintConfig {
    /**
     * The root of the linted project: where the parsers come from, and what a
     * rule resolves the paths of its options and settings from.
     */
    cwd: string;
    /** Every rule entry of the file, hosted or not: `context.fileRules`. */
    fileRules: ReadonlyMap<string, unknown>;
    /** The absolute path of the file. */
    filename: string;
    /** The hosted rules, by name: a severity, or a severity followed by the options. */
    rules: Readonly<Record<string, unknown>>;
    settings: Record<string, unknown>;
    /** The rule of a name, `undefined` for an unknown one. */
    ruleOf: (id: string) => HostedRule | undefined;
}

const BOM = "\uFEFF";
const WARNING: Severity = 1;
const ERROR: Severity = 2;
/** ESLint's severities, and Oxlint's `allow` and `deny`. */
const SEVERITIES = new Map<unknown, Severity | 0>([
    [0, 0],
    [1, WARNING],
    [2, ERROR],
    ["allow", 0],
    ["deny", ERROR],
    ["error", ERROR],
    ["off", 0],
    ["warn", WARNING],
]);
/** As ESLint: a pass may leave fixes that overlap those it applied. */
const MAX_FIX_PASSES = 10;

function severityOf(id: string, entry: unknown): Severity | 0 {
    const severity = SEVERITIES.get(Array.isArray(entry) ? entry[0] : entry);
    if (severity === undefined) {
        throw new ConfigurationError(
            `The severity of ${id} is not one of 0, 1, 2, "off", "allow", "warn", "error" and "deny"`,
        );
    }

    return severity;
}

const optionsOf = (entry: unknown): unknown[] => (Array.isArray(entry) ? entry.slice(1) : []);

const PLACEHOLDER = /\{\{(?<term>[^{}]+?)\}\}/gu;

/** `message` with the `{{placeholders}}` that `data` has a value for. */
const interpolate = (message: string, data: Record<string, unknown> | undefined): string =>
    data === undefined
        ? message
        : message.replaceAll(PLACEHOLDER, (placeholder, term: string) =>
              term.trim() in data ? String(data[term.trim()]) : placeholder,
          );

function messageOf(descriptor: ReportDescriptor, rule: HostedRule): string {
    const { message, messageId } = descriptor;
    if (messageId === undefined) {
        if (message === undefined || message === "") {
            throw new TypeError(
                "Missing `message` property in report() call; add a message that describes the linting problem.",
            );
        }

        return message;
    }
    const text = rule.meta?.messages?.[messageId];
    if (text === undefined) {
        throw new TypeError(
            `context.report() called with a messageId of '${messageId}' which is not present in the 'messages' config of the rule.`,
        );
    }

    return text;
}

/** Where a report is: its `loc`, which may be a position alone, or else its node. */
function locationOf(descriptor: ReportDescriptor): Partial<SourceLocation> & Pick<SourceLocation, "start"> {
    const { loc, node } = descriptor;
    if (loc !== undefined) {
        return "start" in loc ? loc : { start: loc };
    }
    if (node === undefined) {
        throw new TypeError("Node must be provided when reporting error if location is not provided");
    }

    return node.loc;
}

interface RuleRun {
    id: string;
    rule: HostedRule;
    severity: Severity;
}

/** The `report` of a rule context: it adds the reports of the rule to `messages`. */
const reporter =
    ({ id, rule, severity }: RuleRun, text: string, messages: LintMessage[]) =>
    (descriptor: ReportDescriptor): void => {
        const { end, start } = locationOf(descriptor);
        const fix = fixOf(descriptor, text);
        if (fix !== undefined && !rule.meta?.fixable) {
            throw new Error('Fixable rules must set the `meta.fixable` property to "code" or "whitespace".');
        }
        messages.push({
            column: start.column + 1,
            line: start.line,
            message: interpolate(messageOf(descriptor, rule), descriptor.data),
            ruleId: id,
            severity,
            ...(descriptor.messageId === undefined ? {} : { messageId: descriptor.messageId }),
            ...(end === undefined ? {} : { endColumn: end.column + 1, endLine: end.line }),
            ...(fix === undefined ? {} : { fix }),
        });
    };

interface ParseFailure {
    column: number;
    line: number;
    message: string;
}

/** A syntax error as the parsers throw it: an Error with a position. */
function parseFailure(error: unknown): ParseFailure | undefined {
    return error instanceof Error
        && "lineNumber" in error
        && typeof error.lineNumber === "number"
        && "column" in error
        && typeof error.column === "number"
        ? { column: error.column, line: error.lineNumber, message: error.message }
        : undefined;
}

const LINE_PREFIX = /^line \d+:/iu;

function parse(source: string, filename: string, root: string): LintMessage | SourceCode {
    const hasBOM = source.startsWith(BOM);
    const text = hasBOM ? source.slice(BOM.length) : source;
    try {
        const parsed = parseVue(text, filename, root);

        return createSourceCode(
            text,
            {
                ...parsed,
                parserServices: { ...parsed.parserServices, defineCustomBlocksVisitor },
            },
            hasBOM,
        );
    }
    catch (error) {
        const failure = parseFailure(error);
        if (failure === undefined) {
            throw error;
        }

        return {
            column: failure.column,
            fatal: true,
            line: failure.line,
            message: `Parsing error: ${failure.message.replace(LINE_PREFIX, "").trim()}`,
            ruleId: null,
            severity: ERROR,
        };
    }
}

function isMessage(value: LintMessage | SourceCode): value is LintMessage {
    return "ruleId" in value;
}

/** Runs a step of the lint of a file: what it throws says which step, of which file. */
function attempt<T>(step: string, run: () => T): T {
    try {
        return run();
    }
    catch (error) {
        throw new Error(`${step}: ${error instanceof Error ? error.message : String(error)}`, { cause: error });
    }
}

/** The rules of the configuration that are on, in its order. */
function rulesToRun({ ruleOf, rules }: LintConfig): RuleRun[] {
    return Object.entries(rules).flatMap(([id, entry]) => {
        const severity = severityOf(id, entry);
        if (severity === 0) {
            return [];
        }
        const rule = ruleOf(id);
        if (rule === undefined) {
            throw new ConfigurationError(`No plugin has a rule named ${id}`);
        }

        return [{ id, rule, severity }];
    });
}

/**
 * The reports of the rules of `config` on the .vue file `source`, by line
 * and column. A syntax error in a `<script>` block is the only report then,
 * with `fatal`.
 */
export function verify(source: string, config: LintConfig): LintMessage[] {
    const { cwd, fileRules, filename, rules, settings } = config;
    const sourceCode = parse(source, filename, cwd);
    if (isMessage(sourceCode)) {
        return [sourceCode];
    }
    const messages: LintMessage[] = [];
    const listeners: Listeners = new Map();
    for (const run of rulesToRun(config)) {
        const context: RuleContext = {
            cwd,
            fileRules,
            filename,
            id: run.id,
            languageOptions: {
                ecmaVersion: "latest",
                parserOptions: {},
                sourceType: "module",
            },
            options: optionsOf(rules[run.id]),
            physicalFilename: filename,
            report: reporter(run, sourceCode.text, messages),
            settings,
            sourceCode,
        };
        attempt(`${run.id} on ${filename}`, () => {
            addListeners(listeners, run.id, run.rule.create(context));
        });
    }
    attempt(`The lint of ${filename}`, () => {
        walk(sourceCode.ast, sourceCode.visitorKeys, listeners);
    });

    return messages.sort((a, b) => a.line - b.line || a.column - b.column);
}

/**
 * Lints `source` and applies the fixes, pass after pass, until a pass fixes
 * nothing. The messages are those of the fixed text.
 */
export function verifyAndFix(source: string, config: LintConfig): Fixed {
    // The texts of the passes: a pass is circular when it gives the text of two passes before.
    const texts = [source];
    for (let pass = 1; pass <= MAX_FIX_PASSES; pass += 1) {
        const text = texts.at(-1) ?? source;
        const messages = verify(text, config);
        const result = applyFixes(text, messages);
        if (!result.fixed || messages.some(message => message.fatal === true)) {
            return { fixed: texts.length > 1, messages, output: text };
        }
        texts.push(result.output);
        // Two rules that undo each other's fix.
        if (pass > 1 && result.output === texts.at(-3)) {
            break;
        }
    }
    const output = texts.at(-1) ?? source;

    return { fixed: true, messages: verify(output, config), output };
}
