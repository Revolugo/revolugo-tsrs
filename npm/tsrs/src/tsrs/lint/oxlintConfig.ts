/**
 * What an Oxlint configuration file says about a file, for the lint
 * (./cli.ts), which runs the rules of the configuration that Oxlint does
 * not.
 *
 * The rules of a file are the base `rules` and those of each override that
 * matches it, in order, as Oxlint resolves them.
 */
import { readFileSync } from "node:fs";
import {
    dirname,
    matchesGlob,
} from "node:path";
import { isRecord } from "./host/nodes.ts";
import { ConfigurationError } from "./project.ts";

/** A rule's severity alone, or followed by its options. */
export type RuleEntry = unknown;

/** An entry of `jsPlugins`. */
export interface PluginReference {
    /** The alias that the configuration gives to the plugin, instead of the name the plugin has. */
    name?: string;
    /** A path, relative to the directory of the configuration, or what a module of the project imports the plugin with. */
    specifier: string;
}

interface Override {
    excludeFiles: string[];
    files: string[];
    rules: Record<string, RuleEntry>;
}

export interface OxlintConfig {
    /**
     * The directory that the globs of `files`, `excludeFiles` and `ignorePatterns` and the relative paths of
     * `jsPlugins` are resolved against: that of the configuration file, as in Oxlint.
     */
    directory: string;
    ignorePatterns: string[];
    /** The plugins of the configuration and of its overrides, once each. */
    jsPlugins: PluginReference[];
    overrides: Override[];
    rules: Record<string, RuleEntry>;
    settings: Record<string, unknown>;
}

const strings = (value: unknown): string[] => Array.isArray(value) ? value.filter(item => typeof item === "string") : [];

const record = (value: unknown): Record<string, unknown> => isRecord(value) ? value : {};

function pluginReference(path: string, entry: unknown): PluginReference {
    if (typeof entry === "string") {
        return { specifier: entry };
    }
    if (isRecord(entry) && typeof entry["name"] === "string" && typeof entry["specifier"] === "string") {
        return { name: entry["name"], specifier: entry["specifier"] };
    }
    throw new ConfigurationError(
        `${path}: an entry of jsPlugins is a specifier or { "name": ..., "specifier": ... }, not ${JSON.stringify(entry)}`,
    );
}

/** The `jsPlugins` of the configuration and of its overrides, in order, without the repeated ones. */
function pluginReferences(path: string, layers: readonly Record<string, unknown>[]): PluginReference[] {
    const references = new Map<string, PluginReference>();
    for (const layer of layers) {
        const { jsPlugins } = layer;
        for (const entry of Array.isArray(jsPlugins) ? jsPlugins : []) {
            const reference = pluginReference(path, entry);
            references.set(JSON.stringify([reference.specifier, reference.name]), reference);
        }
    }

    return [...references.values()];
}

/**
 * Reads the configuration file at `path`. It applies to `directory`, which is the directory of the file unless the
 * command line names another one (`--root`).
 */
export function readOxlintConfig(path: string, directory: string = dirname(path)): OxlintConfig {
    let config: unknown;
    try {
        config = JSON.parse(readFileSync(path, "utf8"));
    }
    catch (error) {
        throw new ConfigurationError(
            `Cannot read the configuration ${path}: ${error instanceof Error ? error.message : String(error)}`,
            { cause: error },
        );
    }
    if (!isRecord(config) || Array.isArray(config)) {
        throw new ConfigurationError(`${path} is not an object`);
    }
    const overrides: unknown[] = Array.isArray(config["overrides"]) ? config["overrides"] : [];
    const layers = overrides.filter(isRecord);

    return {
        directory,
        ignorePatterns: strings(config["ignorePatterns"]),
        jsPlugins: pluginReferences(path, [config, ...layers]),
        overrides: layers.map(override => ({
            excludeFiles: strings(override["excludeFiles"]),
            files: strings(override["files"]),
            rules: record(override["rules"]),
        })),
        rules: record(config["rules"]),
        settings: record(config["settings"]),
    };
}

/** What a glob is made of when it compiles to a regular expression: no escape, class, range or extglob. */
const SIMPLE_GLOB = /^[^\\[\]()!+@]*$/u;
const REGEXP_SPECIAL = /[.$^|]/gu;

/** The alternatives of the first `{a,b}` of `glob`, expanded in turn, or `glob` itself when it has none. */
function expandBraces(glob: string): string[] | undefined {
    const open = glob.indexOf("{");
    if (open === -1) {
        return glob.includes("}") ? undefined : [glob];
    }
    const close = glob.indexOf("}", open);
    const body = glob.slice(open + 1, close);
    // Nested braces, `{1..3}` and a `{a}` without a comma are for `matchesGlob`.
    if (close === -1 || body.includes("{") || body.includes("..") || !body.includes(",")) {
        return undefined;
    }
    const alternatives: string[] = [];
    for (const part of body.split(",")) {
        const expanded = expandBraces(glob.slice(0, open) + part + glob.slice(close + 1));
        if (expanded === undefined) {
            return undefined;
        }
        alternatives.push(...expanded);
    }

    return alternatives;
}

/** The source of the regular expression of a glob without braces: a wildcard matches no leading dot of a segment. */
function segmentsSource(glob: string): string | undefined {
    const segments = glob.split("/");
    if (segments.some(segment => segment === "" || segment === "." || segment === "..")) {
        return undefined;
    }
    let source = "";
    for (const [index, segment] of segments.entries()) {
        const last = index === segments.length - 1;
        if (segment === "**") {
            // Any number of segments, and at the end at least one.
            source += last ? "(?:(?!\\.)[^/]+/)*(?!\\.)[^/]+" : "(?:(?!\\.)[^/]+/)*";
            continue;
        }
        const body = segment
            .replaceAll(REGEXP_SPECIAL, String.raw`\$&`)
            .replaceAll(/\*+/gu, "[^/]*")
            .replaceAll("?", "[^/]");
        // `*` alone matches no empty segment.
        source += `${segment.startsWith(".") ? "" : "(?!\\.)"}${/^\*+$/u.test(segment) ? "[^/]+" : body}${last ? "" : "/"}`;
    }

    return source;
}

const matchers = new Map<string, (file: string) => boolean>();

/**
 * What tells whether a file matches `glob`, as `matchesGlob` does. A lint asks about every file and every glob of
 * the configuration: a glob that compiles to a regular expression is compiled once.
 */
function matcher(glob: string): (file: string) => boolean {
    let matches = matchers.get(glob);
    if (matches === undefined) {
        const sources = SIMPLE_GLOB.test(glob) ? expandBraces(glob)?.map(segmentsSource) : undefined;
        if (sources === undefined || sources.includes(undefined)) {
            matches = file => matchesGlob(file, glob);
        }
        else {
            const pattern = new RegExp(`^(?:${sources.join("|")})$`, "u");
            // A path that is not plain segments is for `matchesGlob` too.
            matches = file => PLAIN_PATH.test(file) ? pattern.test(file) : matchesGlob(file, glob);
        }
        matchers.set(glob, matches);
    }

    return matches;
}

/** A relative path of segments that are neither empty, `.` nor `..`. */
const PLAIN_PATH = /^(?!\.{1,2}(?:\/|$))[^/\\]+(?:\/(?!\.{1,2}(?:\/|$))[^/\\]+)*$/u;

/** Whether `file` matches `glob`, as `matchesGlob` says. */
export const matchesPattern = (file: string, glob: string): boolean => matcher(glob)(file);

const matches = (file: string, patterns: readonly string[]): boolean => patterns.some(pattern => matchesPattern(file, pattern));

/** Whether `ignorePatterns` leaves `file` (relative to the directory of the configuration) out. */
export const isIgnored = (config: OxlintConfig, file: string): boolean =>
    config.ignorePatterns.some(
        pattern =>
            matchesPattern(file, pattern)
            || matchesPattern(file, `${pattern}/**`)
            || matchesPattern(file, `**/${pattern}`)
            || matchesPattern(file, `**/${pattern}/**`),
    );

const OFF = new Set<unknown>([0, "allow", "off"]);

/** The severity of a rule entry, with or without options. */
export const severityOf = (entry: RuleEntry): unknown => Array.isArray(entry) ? entry[0] : entry;

/** The options of a rule entry: what follows its severity. */
export const optionsOf = (entry: RuleEntry): unknown[] => Array.isArray(entry) ? entry.slice(1) : [];

export const isOff = (entry: RuleEntry): boolean => OFF.has(severityOf(entry));

/**
 * The rules of `file` (relative to the directory of the configuration), by
 * the name they have in the configuration. As in Oxlint, a severity alone
 * keeps the options that an earlier layer gave to the rule.
 */
export function rulesFor(config: OxlintConfig, file: string): Map<string, RuleEntry> {
    const rules = new Map<string, RuleEntry>();
    const apply = (layer: Record<string, RuleEntry>): void => {
        for (const [name, entry] of Object.entries(layer)) {
            const earlier = rules.get(name);
            rules.set(
                name,
                Array.isArray(entry) || isOff(entry) || earlier === undefined
                    ? entry
                    : [entry, ...optionsOf(earlier)],
            );
        }
    };
    apply(config.rules);
    for (const override of config.overrides) {
        if (matches(file, override.files) && !matches(file, override.excludeFiles)) {
            apply(override.rules);
        }
    }

    return rules;
}
