/**
 * What an Oxlint configuration file says about a file, for the Vue lint
 * (./cli.ts): Oxlint only lints the `<script>` blocks of a .vue file, so the
 * Vue lint applies the rest of the configuration itself.
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

const matches = (file: string, patterns: readonly string[]): boolean => patterns.some(pattern => matchesGlob(file, pattern));

/** Whether `ignorePatterns` leaves `file` (relative to the directory of the configuration) out. */
export const isIgnored = (config: OxlintConfig, file: string): boolean =>
    config.ignorePatterns.some(
        pattern =>
            matchesGlob(file, pattern)
            || matchesGlob(file, `${pattern}/**`)
            || matchesGlob(file, `**/${pattern}`)
            || matchesGlob(file, `**/${pattern}/**`),
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
