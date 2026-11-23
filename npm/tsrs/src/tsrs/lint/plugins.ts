/**
 * The JS plugins of the configuration (`jsPlugins`), and the rules of those
 * that the lint runs in place of Oxlint.
 *
 * A plugin is what Oxlint loads: an ES module whose default export is
 * `{ meta: { name }, rules }`. Oxlint runs its rules on the scripts it
 * parses. Two things in `meta` give a rule to the lint:
 *
 * - `sfc: true` is a plugin whose rules read a whole .vue file, the
 *   `<template>` and the custom blocks included: Oxlint cannot run them, and
 *   the rule host does (./host/host.ts), for the .vue files only;
 * - `native: ["camelcase"]` names the rules that `tsrs-lint` has a port of.
 *   The plugin does not run them on a script file, and `tsrs-lint` does
 *   (./tsrsLint.ts). On the `<script>` blocks of a .vue file, which
 *   `tsrs-lint` sees only as what the content mapper makes of them, the JS
 *   rule still runs, in Oxlint.
 */
import {
    existsSync,
    readFileSync,
} from "node:fs";
import { createRequire } from "node:module";
import {
    dirname,
    isAbsolute,
    join,
    resolve,
} from "node:path";
import { pathToFileURL } from "node:url";
import { isRecord } from "./host/nodes.ts";
import type { HostedRule } from "./host/types.ts";
import type {
    OxlintConfig,
    PluginReference,
    RuleEntry,
} from "./oxlintConfig.ts";
import { ConfigurationError } from "./project.ts";

export interface Plugin {
    /** What the configuration names the rules of the plugin with: `<name>/<rule>`. */
    name: string;
    /** The rules that `tsrs-lint` runs on the script files, by the name `tsrs-lint` has for each. */
    native: Map<string, string>;
    rules: Record<string, unknown>;
    /** Whether the rules read a whole .vue file: the rule host runs them. */
    sfc: boolean;
    /** The `jsPlugins` entry the plugin comes from. */
    specifier: string;
}

const RELATIVE = /^\.{1,2}(?:[/\\]|$)/u;
const PACKAGE = /^(?<name>(?:@[^/]+\/)?[^/]+)(?<subpath>\/.*)?$/u;
/** The conditions of an `import`, as Node resolves one. */
const CONDITIONS = new Set(["default", "import", "module-sync", "node"]);

function codeOf(error: unknown): unknown {
    return isRecord(error) ? error["code"] : undefined;
}

/** The first target of a package's `exports` entry that an `import` takes. */
function importTarget(entry: unknown): string | undefined {
    if (typeof entry === "string") {
        return entry;
    }
    if (Array.isArray(entry)) {
        return entry.map(importTarget).find(target => target !== undefined);
    }
    if (isRecord(entry)) {
        for (const [condition, target] of Object.entries(entry)) {
            const found = CONDITIONS.has(condition) ? importTarget(target) : undefined;
            if (found !== undefined) {
                return found;
            }
        }
    }

    return undefined;
}

/**
 * The file of a package that only an `import` can load: `require.resolve` rejects a package whose `exports` have no
 * `require` or `default` condition. The package is looked for in the `node_modules` of `root` and of its parents.
 */
function importOnlyFile(root: string, specifier: string): string | undefined {
    const { name, subpath } = PACKAGE.exec(specifier)?.groups ?? {};
    if (name === undefined) {
        return undefined;
    }
    for (let directory = root;; directory = dirname(directory)) {
        const manifest = join(directory, "node_modules", name, "package.json");
        if (existsSync(manifest)) {
            const exported: unknown = JSON.parse(readFileSync(manifest, "utf8")).exports;
            const key = `.${subpath ?? ""}`;
            const entry = isRecord(exported) && !Array.isArray(exported) && Object.keys(exported).some(k => k.startsWith("."))
                ? exported[key]
                : key === "." ? exported : undefined;
            const target = importTarget(entry);

            return target === undefined ? undefined : join(dirname(manifest), target);
        }
        if (directory === dirname(directory)) {
            return undefined;
        }
    }
}

/** The file that a module of the project at `root` gets for the package `specifier`. */
function packageFile(root: string, specifier: string): string {
    try {
        return createRequire(join(root, "package.json")).resolve(specifier);
    }
    catch (error) {
        const file = codeOf(error) === "ERR_PACKAGE_PATH_NOT_EXPORTED" ? importOnlyFile(root, specifier) : undefined;
        if (file === undefined) {
            throw new ConfigurationError(
                `Cannot find the plugin ${specifier} from ${root}: ${error instanceof Error ? error.message : String(error)}`,
                { cause: error },
            );
        }

        return file;
    }
}

/**
 * Where a `jsPlugins` specifier is, as Oxlint finds it: a path is relative to the directory of the configuration,
 * and anything else is a package of the project at `root`.
 */
function urlOf(specifier: string, directory: string, root: string): string {
    if (specifier.startsWith("file:")) {
        return specifier;
    }
    const file = RELATIVE.test(specifier) || isAbsolute(specifier)
        ? resolve(directory, specifier)
        : packageFile(root, specifier);

    return pathToFileURL(file).href;
}

const PLUGIN_PREFIXES = ["eslint-plugin", "oxlint-plugin"];

/**
 * The name of a plugin without what says that it is one, as Oxlint normalizes it: `eslint-plugin-foo` is `foo`,
 * `@scope/eslint-plugin` is `@scope`, and `@scope/eslint-plugin-foo` is `@scope/foo`.
 */
export function normalizePluginName(name: string): string {
    const slash = name.indexOf("/");
    const scope = slash === -1 ? "" : name.slice(0, slash);
    const rest = name.slice(slash + 1);
    for (const prefix of PLUGIN_PREFIXES) {
        if (rest === prefix && scope !== "") {
            return scope;
        }
        if (rest.startsWith(`${prefix}-`)) {
            const short = rest.slice(prefix.length + 1);

            return scope === "" ? short : `${scope}/${short}`;
        }
    }

    return name;
}

/** The name of a plugin: its alias, or else the name it gives itself, or else that of its package. */
function nameOf({ name, specifier }: PluginReference, meta: Record<string, unknown>): string {
    if (name !== undefined) {
        return name;
    }
    if (meta["name"] !== undefined && meta["name"] !== null) {
        if (typeof meta["name"] !== "string") {
            throw new ConfigurationError(`The meta.name of the plugin ${specifier} is not a string`);
        }

        return normalizePluginName(meta["name"]);
    }
    const packageName = RELATIVE.test(specifier) || isAbsolute(specifier) || specifier.startsWith("file:")
        ? undefined
        : PACKAGE.exec(specifier)?.groups?.["name"];
    if (packageName === undefined) {
        throw new ConfigurationError(
            `The plugin ${specifier} has no name: it defines no meta.name, is not a package, and has no alias in jsPlugins`,
        );
    }

    return normalizePluginName(packageName);
}

/**
 * The rules of `meta.native`, each with the name `tsrs-lint` has for it: `<plugin>/<rule>`, where the plugin is named
 * as it names itself, whatever alias the configuration gives it.
 */
function nativeRules({ specifier }: PluginReference, meta: Record<string, unknown>, name: string): Map<string, string> {
    const { native } = meta;
    if (native === undefined) {
        return new Map();
    }
    if (!Array.isArray(native) || native.some(rule => typeof rule !== "string")) {
        throw new ConfigurationError(`The meta.native of the plugin ${specifier} is not an array of rule names`);
    }
    const own = typeof meta["name"] === "string" ? normalizePluginName(meta["name"]) : name;

    return new Map(native.map((rule: string) => [rule, `${own}/${rule}`]));
}

async function loadPlugin(reference: PluginReference, config: OxlintConfig, root: string): Promise<Plugin> {
    const { specifier } = reference;
    const url = urlOf(specifier, config.directory, root);
    let loaded: unknown;
    try {
        loaded = await import(url);
    }
    catch (error) {
        throw new ConfigurationError(
            `Cannot load the plugin ${specifier} (${url}): ${error instanceof Error ? error.message : String(error)}`,
            { cause: error },
        );
    }
    const plugin = isRecord(loaded) ? loaded["default"] : undefined;
    if (!isRecord(plugin)) {
        throw new ConfigurationError(`The plugin ${specifier} (${url}) has no default export that is a plugin`);
    }
    const meta = isRecord(plugin["meta"]) ? plugin["meta"] : {};

    const name = nameOf(reference, meta);

    return {
        name,
        native: nativeRules(reference, meta, name),
        rules: isRecord(plugin["rules"]) ? plugin["rules"] : {},
        sfc: meta["sfc"] === true,
        specifier,
    };
}

/** Loads the plugins of the configuration, in its order, for the project at `root`. */
export async function loadPlugins(config: OxlintConfig, root: string): Promise<Plugin[]> {
    const plugins: Plugin[] = [];
    for (const reference of config.jsPlugins) {
        const plugin = await loadPlugin(reference, config, root);
        const same = plugins.find(other => other.name === plugin.name);
        if (same !== undefined) {
            throw new ConfigurationError(
                `The plugins ${same.specifier} and ${plugin.specifier} are both named ${plugin.name}: give one an alias in jsPlugins`,
            );
        }
        plugins.push(plugin);
    }

    return plugins;
}

/**
 * The name `tsrs-lint` has for each rule that a plugin leaves to it (`meta.native`), by the name the configuration
 * has for the rule.
 */
export const nativeRuleNames = (plugins: readonly Plugin[]): Map<string, string> =>
    new Map(plugins.flatMap(plugin => [...plugin.native].map(([rule, native]) => [`${plugin.name}/${rule}`, native])));

function isHostedRule(value: unknown): value is HostedRule {
    return isRecord(value) && typeof value["create"] === "function";
}

/** The rules that the rule host runs: those of the plugins with `meta.sfc`. */
export interface HostedRules {
    /** The entries of `fileRules` that name a hosted rule, in their order. */
    entriesOf: (fileRules: ReadonlyMap<string, RuleEntry>) => Record<string, RuleEntry>;
    /** The rule that a configuration names, `undefined` when its plugin has none of this name. */
    ruleOf: (id: string) => HostedRule | undefined;
}

export function hostedRules(plugins: readonly Plugin[]): HostedRules {
    // The longest name first: `@scope/foo/rule` is a rule of `@scope/foo` before one of `@scope`.
    const hosting = plugins.filter(plugin => plugin.sfc).sort((a, b) => b.name.length - a.name.length);
    const pluginOf = (id: string): Plugin | undefined => hosting.find(plugin => id.startsWith(`${plugin.name}/`));

    return {
        entriesOf: fileRules =>
            hosting.length === 0
                ? {}
                : Object.fromEntries([...fileRules].filter(([id]) => pluginOf(id) !== undefined)),
        ruleOf(id) {
            const plugin = pluginOf(id);
            if (plugin === undefined) {
                return undefined;
            }
            const rule = plugin.rules[id.slice(plugin.name.length + 1)];

            return isHostedRule(rule) ? rule : undefined;
        },
    };
}
