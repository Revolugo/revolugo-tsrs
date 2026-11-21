// tsrs-lint-vue: how it reads an Oxlint configuration, and how it loads the JS plugins that the configuration names.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import {
    isIgnored,
    isOff,
    optionsOf,
    type OxlintConfig,
    readOxlintConfig,
    rulesFor,
} from "../../src/tsrs/lintVue/oxlintConfig.ts";
import {
    hostedRules,
    loadPlugins,
    normalizePluginName,
} from "../../src/tsrs/lintVue/plugins.ts";
import { ConfigurationError } from "../../src/tsrs/lintVue/project.ts";
import { createProject } from "./lint-vue.testUtils.ts";

const config = (overrides: OxlintConfig["overrides"]): OxlintConfig => ({
    directory: "/project",
    ignorePatterns: ["**/dist", "libs/generated"],
    jsPlugins: [],
    overrides,
    rules: { "no-console": [1, { allow: ["warn"] }], "no-var": 2 },
    settings: {},
});

test("a file gets the base rules and those of each matching override, in order", () => {
    const rules = rulesFor(
        config([
            { excludeFiles: [], files: ["**/*.vue"], rules: { "no-console": 2, "no-var": 0 } },
            { excludeFiles: ["**/Legacy.vue"], files: ["apps/**/*.vue"], rules: { "vue/x": [2, "always"] } },
            { excludeFiles: [], files: ["apps/**/*.vue"], rules: { "no-console": "off", "vue/x": "warn" } },
            { excludeFiles: [], files: ["libs/**"], rules: { "vue/x": 0 } },
        ]),
        "apps/front/App.vue",
    );

    // A severity alone keeps the options of the earlier layer, unless it turns the rule off.
    assert.deepEqual(rules.get("vue/x"), ["warn", "always"]);
    assert.equal(rules.get("no-console"), "off");
    assert.equal(isOff(rules.get("no-var")), true);
    assert.deepEqual(optionsOf(rules.get("vue/x")), ["always"]);
    assert.deepEqual([...rules.keys()], ["no-console", "no-var", "vue/x"]);
});

test("a severity alone keeps the options of the base rules", () => {
    const rules = rulesFor(
        config([{ excludeFiles: [], files: ["**/*.vue"], rules: { "no-console": 2 } }]),
        "apps/front/App.vue",
    );

    assert.deepEqual(rules.get("no-console"), [2, { allow: ["warn"] }]);
});

test("an override does not apply to the files it excludes", () => {
    const rules = rulesFor(
        config([{ excludeFiles: ["**/Legacy.vue"], files: ["apps/**/*.vue"], rules: { "vue/x": 2 } }]),
        "apps/front/Legacy.vue",
    );

    assert.equal(rules.has("vue/x"), false);
});

test("ignorePatterns leave out a directory at any depth and a rooted path", () => {
    const ignoring = config([]);

    assert.equal(isIgnored(ignoring, "apps/front/dist/App.vue"), true);
    assert.equal(isIgnored(ignoring, "libs/generated/App.vue"), true);
    assert.equal(isIgnored(ignoring, "apps/front/src/App.vue"), false);
});

test("the configuration file gives its directory, its rules and the plugins of every layer once", t => {
    const root = createProject(t, {
        "config/.oxlintrc.json": {
            ignorePatterns: ["generated"],
            jsPlugins: ["./a.mjs", { name: "alias", specifier: "eslint-plugin-b" }],
            overrides: [
                { files: ["**/*.vue"], jsPlugins: ["./a.mjs", "./c.mjs"], rules: { "a/x": 1 } },
                "not an override",
            ],
            rules: { "a/x": 2 },
            settings: { thing: { key: "value" } },
        },
    }, []);
    const file = path.join(root, "config/.oxlintrc.json");
    const read = readOxlintConfig(file);

    assert.deepEqual(read, {
        directory: path.join(root, "config"),
        ignorePatterns: ["generated"],
        jsPlugins: [{ specifier: "./a.mjs" }, { name: "alias", specifier: "eslint-plugin-b" }, { specifier: "./c.mjs" }],
        overrides: [{ excludeFiles: [], files: ["**/*.vue"], rules: { "a/x": 1 } }],
        rules: { "a/x": 2 },
        settings: { thing: { key: "value" } },
    });
    // `--root`: the configuration applies to another directory than its own.
    assert.equal(readOxlintConfig(file, root).directory, root);
});

test("a configuration that cannot be read, or whose jsPlugins are not plugins, is a configuration error", t => {
    const root = createProject(t, {
        "broken.json": "{ rules: ",
        "list.json": "[]",
        "plugins.json": { jsPlugins: [{ specifier: "./a.mjs" }] },
    }, []);

    assert.throws(() => readOxlintConfig(path.join(root, "missing.json")), (error: unknown) => {
        assert.ok(error instanceof ConfigurationError);
        assert.match(error.message, /^Cannot read the configuration .*missing\.json: ENOENT/u);
        return true;
    });
    assert.throws(() => readOxlintConfig(path.join(root, "broken.json")), ConfigurationError);
    assert.throws(() => readOxlintConfig(path.join(root, "list.json")), /list\.json is not an object/u);
    assert.throws(() => readOxlintConfig(path.join(root, "plugins.json")), /an entry of jsPlugins is a specifier or/u);
});

test("a plugin name loses what says that it is a plugin, as in Oxlint", () => {
    assert.deepEqual(
        [
            "eslint-plugin-foo",
            "oxlint-plugin-foo",
            "@scope/eslint-plugin",
            "@scope/eslint-plugin-foo",
            "@scope/oxlint-plugin-foo",
            "foo",
            "@scope/foo",
            "eslint-plugin",
        ].map(normalizePluginName),
        ["foo", "foo", "@scope", "@scope/foo", "@scope/foo", "foo", "@scope/foo", "eslint-plugin"],
    );
});

const plugin = (meta: object, rule = "x"): string => `export default { meta: ${JSON.stringify(meta)}, rules: { ${JSON.stringify(rule)}: { create: () => ({}) } } };\n`;

test("a plugin is a path from the configuration's directory or a package of the project, under its alias, its name or its package's", async t => {
    const root = createProject(t, {
        "config/.oxlintrc.json": {
            jsPlugins: [
                "./plugins/local.mjs",
                "../typed.ts",
                "eslint-plugin-unnamed",
                "@scope/eslint-plugin-named/sub",
                "import-only",
                { name: "alias", specifier: "eslint-plugin-unnamed/other.js" },
            ],
        },
        "config/plugins/local.mjs": plugin({ name: "local", sfc: true }),
        // A plugin in TypeScript: Node strips the types.
        "typed.ts": `const name: string = "typed";\nexport default { meta: { name }, rules: {} };\n`,
        "node_modules/eslint-plugin-unnamed/package.json": { main: "./index.js", name: "eslint-plugin-unnamed" },
        // CommonJS: the plugin is the default export of the module that \`import()\` returns.
        "node_modules/eslint-plugin-unnamed/index.js": "module.exports = { rules: { y: { create() { return {}; } } } };\n",
        "node_modules/eslint-plugin-unnamed/other.js": "module.exports = { meta: { name: 'ignored' }, rules: {} };\n",
        "node_modules/@scope/eslint-plugin-named/package.json": {
            exports: { "./sub": "./sub.mjs" },
            name: "@scope/eslint-plugin-named",
        },
        "node_modules/@scope/eslint-plugin-named/sub.mjs": plugin({ name: "eslint-plugin-renamed", sfc: false }),
        // No `require` or `default` condition: `require.resolve` does not find it, `import` does.
        "node_modules/import-only/package.json": {
            exports: { ".": { types: "./index.d.ts", import: "./index.js" } },
            name: "import-only",
            type: "module",
        },
        "node_modules/import-only/index.js": plugin({ name: "import-only", sfc: true }, "z"),
    }, []);
    const plugins = await loadPlugins(readOxlintConfig(path.join(root, "config/.oxlintrc.json")), root);

    assert.deepEqual(
        plugins.map(({ name, rules, sfc, specifier }) => ({ name, rules: Object.keys(rules), sfc, specifier })),
        [
            { name: "local", rules: ["x"], sfc: true, specifier: "./plugins/local.mjs" },
            { name: "typed", rules: [], sfc: false, specifier: "../typed.ts" },
            { name: "unnamed", rules: ["y"], sfc: false, specifier: "eslint-plugin-unnamed" },
            { name: "renamed", rules: ["x"], sfc: false, specifier: "@scope/eslint-plugin-named/sub" },
            { name: "import-only", rules: ["z"], sfc: true, specifier: "import-only" },
            { name: "alias", rules: [], sfc: false, specifier: "eslint-plugin-unnamed/other.js" },
        ],
    );

    // The rule host runs the rules of the plugins with `meta.sfc`, and no other.
    const hosted = hostedRules(plugins);
    assert.deepEqual(
        hosted.entriesOf(
            new Map<string, unknown>([["no-var", 2], ["local/x", [1, "option"]], ["unnamed/y", 2], ["import-only/z", 0], ["local/unknown", 2]]),
        ),
        { "local/x": [1, "option"], "import-only/z": 0, "local/unknown": 2 },
    );
    assert.equal(typeof hosted.ruleOf("local/x")?.create, "function");
    assert.equal(hosted.ruleOf("local/unknown"), undefined);
    assert.equal(hosted.ruleOf("unnamed/y"), undefined);
});

test("a plugin that cannot be found, loaded or named is a configuration error", async t => {
    const root = createProject(t, {
        "anonymous.mjs": "export default { rules: {} };\n",
        "first.mjs": plugin({ name: "same" }),
        "second.mjs": plugin({ name: "eslint-plugin-same" }),
        "throws.mjs": "throw new Error('boom');\n",
        "empty.mjs": "export const rules = {};\n",
    }, []);
    const loading = (...jsPlugins: unknown[]): Promise<unknown> => {
        fs.writeFileSync(path.join(root, ".oxlintrc.json"), JSON.stringify({ jsPlugins }));
        return loadPlugins(readOxlintConfig(path.join(root, ".oxlintrc.json")), root);
    };
    const rejects = (promise: Promise<unknown>, message: RegExp): Promise<void> =>
        assert.rejects(promise, (error: unknown) => {
            assert.ok(error instanceof ConfigurationError, String(error));
            assert.match(error.message, message);
            return true;
        });

    await rejects(loading("eslint-plugin-missing"), /^Cannot find the plugin eslint-plugin-missing from /u);
    await rejects(loading("./missing.mjs"), /^Cannot load the plugin \.\/missing\.mjs \(file:.*missing\.mjs\): /u);
    await rejects(loading("./throws.mjs"), /^Cannot load the plugin \.\/throws\.mjs .*: boom$/u);
    await rejects(loading("./empty.mjs"), /has no default export that is a plugin$/u);
    await rejects(loading("./anonymous.mjs"), /^The plugin \.\/anonymous\.mjs has no name/u);
    await rejects(loading("./first.mjs", "./second.mjs"), /are both named same: give one an alias/u);
    assert.deepEqual(
        (await loadPlugins(
            { ...config([]), directory: root, jsPlugins: [{ specifier: "./first.mjs" }, { name: "other", specifier: "./second.mjs" }, { name: "anonymous", specifier: "./anonymous.mjs" }] },
            root,
        )).map(loaded => loaded.name),
        ["same", "other", "anonymous"],
    );
});
