// tsrs-lint: the rule host, which does for a rule on a .vue file what ESLint's `Linter` does. The parsers are
// those of npm/lint-vue-project, where the `eslint` package is not installed.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import path from "node:path";
import test from "node:test";
import {
    type LintConfig,
    verify,
    verifyAndFix,
} from "../../src/tsrs/lint/host/host.ts";
import type {
    AstNode,
    HostedRule,
    LintMessage,
    RuleContext,
} from "../../src/tsrs/lint/host/types.ts";
import { ConfigurationError } from "../../src/tsrs/lint/project.ts";
import { PROJECT } from "./lint.testUtils.ts";

const FILE = path.join(PROJECT, "src/components/Sample.vue");

/** A configuration that runs `rules`, each at the severity or with the entry of `entries`. */
const configFor = (
    rules: Readonly<Record<string, HostedRule>>,
    entries: Readonly<Record<string, unknown>> = Object.fromEntries(Object.keys(rules).map(id => [id, 2])),
    more: Partial<LintConfig> = {},
): LintConfig => ({
    cwd: PROJECT,
    fileRules: new Map(Object.entries(entries)),
    filename: FILE,
    ruleOf: id => rules[id],
    rules: entries,
    settings: {},
    ...more,
});

const format = (messages: readonly LintMessage[]): string[] => messages.map(message => `${message.line}:${message.column} ${message.message}`);

const template = (content: string): string => `<template>\n  ${content}\n</template>\n`;

/** A rule that reports the `v-html` directives of the template. */
const noVHtml: HostedRule = {
    create: context =>
        context.sourceCode.parserServices.defineTemplateBodyVisitor({
            "VAttribute[directive=true][key.name.name='html']"(node: AstNode) {
                context.report({ message: "'v-html' directive can lead to XSS attack.", node });
            },
        }),
};

test("the parsers of the project run without the eslint package", () => {
    assert.throws(() => createRequire(path.join(PROJECT, "package.json")).resolve("eslint"), { code: "MODULE_NOT_FOUND" });
    assert.deepEqual(format(verify(template('<div v-html="html" />'), configFor({ "vue/no-v-html": noVHtml }))), [
        "2:8 'v-html' directive can lead to XSS attack.",
    ]);
});

test("a rule reports at its severity, which is ESLint's or Oxlint's", () => {
    const source = template('<div v-html="html" />');
    const severities = (entry: unknown): number[] => verify(source, configFor({ "vue/no-v-html": noVHtml }, { "vue/no-v-html": entry })).map(message => message.severity);

    assert.deepEqual([1, "warn", [1, "option"]].flatMap(severities), [1, 1, 1]);
    assert.deepEqual([2, "error", "deny", ["error"]].flatMap(severities), [2, 2, 2, 2]);
    assert.deepEqual([0, "off", "allow", ["off", "option"]].flatMap(severities), []);
    assert.throws(() => severities("loud"), (error: unknown) => {
        assert.ok(error instanceof ConfigurationError);
        assert.match(error.message, /^The severity of vue\/no-v-html is not one of 0, 1, 2/u);
        return true;
    });
});

test("a message has the rule, the location of the node and the range of the report", () => {
    const [message, ...others] = verify(
        template('<div v-html="html" />'),
        configFor({ "vue/no-v-html": noVHtml }, { "vue/no-v-html": 1 }),
    );

    assert.deepEqual(others, []);
    assert.deepEqual(message, {
        column: 8,
        endColumn: 21,
        endLine: 2,
        line: 2,
        message: "'v-html' directive can lead to XSS attack.",
        ruleId: "vue/no-v-html",
        severity: 1,
    });
});

/** A rule that reports every `old` identifier of the script and of the template, with a fix that renames it. */
const renameOld: HostedRule = {
    create(context) {
        const report = (node: AstNode): void => {
            if (context.sourceCode.getText(node) === "old") {
                context.report({
                    data: { name: context.sourceCode.getText(node) },
                    fix: fixer => fixer.replaceText(node, "renamed"),
                    messageId: "call",
                    node,
                });
            }
        };

        return context.sourceCode.parserServices.defineTemplateBodyVisitor(
            { Identifier: report },
            { Identifier: report },
        );
    },
    meta: { fixable: "code", messages: { call: "Rename {{ name }}." } },
};

test("the host calls the script listeners by node type and the template listeners by selector, and applies the fixes", () => {
    const config = configFor({ "test/rename": renameOld }, { "test/rename": 1 });
    const source = '<script setup lang="ts">\nold()\n</script>\n\n<template>\n  <p>{{ old() }}</p>\n</template>\n';

    assert.deepEqual(
        verify(source, config).map(message => ({
            column: message.column,
            fix: message.fix,
            line: message.line,
            message: message.message,
            ruleId: message.ruleId,
            severity: message.severity,
        })),
        [
            {
                column: 1,
                fix: { range: [25, 28], text: "renamed" },
                line: 2,
                message: "Rename old.",
                ruleId: "test/rename",
                severity: 1,
            },
            {
                column: 9,
                fix: { range: [61, 64], text: "renamed" },
                line: 6,
                message: "Rename old.",
                ruleId: "test/rename",
                severity: 1,
            },
        ],
    );
    const fixed = verifyAndFix(source, config);
    assert.equal(fixed.fixed, true);
    assert.equal(fixed.output, source.replaceAll("old()", "renamed()"));
    assert.deepEqual(fixed.messages, []);
});

/** Two rules whose fixes overlap: each pass applies the first one, and leaves the other for the next pass. */
const swapAttributes = (first: string, second: string): HostedRule => ({
    create: context =>
        context.sourceCode.parserServices.defineTemplateBodyVisitor({
            VStartTag(node: AstNode) {
                const attributes = (node["attributes"] as AstNode[]).map(attribute => ({
                    attribute,
                    name: context.sourceCode.getText(attribute),
                }));
                const before = attributes.findIndex(({ name }) => name === second);
                const after = attributes.findIndex(({ name }) => name === first);
                if (before !== -1 && after > before) {
                    context.report({
                        *fix(fixer) {
                            yield fixer.replaceText(attributes[before]!.attribute, first);
                            yield fixer.replaceText(attributes[after]!.attribute, second);
                        },
                        message: `${first} goes before ${second}.`,
                        node: attributes[after]!.attribute,
                    });
                }
            },
        }),
    meta: { fixable: "code" },
});

test("the fixes of a report are merged, and those that overlap wait for the next pass", () => {
    const rules = { "test/a-b": swapAttributes("a", "b"), "test/b-c": swapAttributes("b", "c") };
    const source = template("<div c b a />");

    assert.deepEqual(format(verify(source, configFor(rules))), ["2:10 b goes before c.", "2:12 a goes before b."]);
    const fixed = verifyAndFix(source, configFor(rules));
    assert.equal(fixed.output, template("<div a b c />"));
    assert.deepEqual(fixed.messages, []);
});

test("a rule that reports a fix without meta.fixable throws", () => {
    assert.throws(
        () => verify(template("<p>{{ old }}</p>"), configFor({ "test/rename": { create: renameOld.create, meta: { messages: { call: "x" } } } })),
        /Fixable rules must set the `meta\.fixable` property/u,
    );
});

test("the host rejects a rule that listens to an esquery selector in the script", () => {
    const rule: HostedRule = {
        create: () => ({ "CallExpression > Identifier": () => undefined }),
    };

    assert.throws(
        () => verify(template("<div />"), configFor({ "test/selector": rule })),
        /listens to the selector "CallExpression > Identifier"/u,
    );
});

test("a rule gets the entries of every rule of the file, its own options, the settings and the project root", () => {
    const seen: Partial<RuleContext>[] = [];
    const rule: HostedRule = {
        create(context) {
            seen.push({
                cwd: context.cwd,
                filename: context.filename,
                id: context.id,
                options: context.options,
                physicalFilename: context.physicalFilename,
                settings: context.settings,
            });

            return {
                Program(node: AstNode) {
                    // What a rule of the template takes from the rules of the script.
                    const syntax = context.fileRules.get("no-restricted-syntax");
                    context.report({ message: `${context.fileRules.size} rules, ${JSON.stringify(syntax)}`, node });
                },
            };
        },
    };
    const fileRules = new Map<string, unknown>([
        ["no-restricted-syntax", [2, "WithStatement"]],
        ["no-var", 0],
        ["test/context", [2, "first", { second: true }]],
    ]);
    const messages = verify(
        template("<div />"),
        configFor({ "test/context": rule }, { "test/context": [2, "first", { second: true }] }, {
            fileRules,
            settings: { "vue-i18n": { localeDir: "./locales/*.json" } },
        }),
    );

    assert.deepEqual(format(messages), ['1:1 3 rules, [2,"WithStatement"]']);
    assert.deepEqual(seen, [{
        cwd: PROJECT,
        filename: FILE,
        id: "test/context",
        options: ["first", { second: true }],
        physicalFilename: FILE,
        settings: { "vue-i18n": { localeDir: "./locales/*.json" } },
    }]);
});

test("a rule that no plugin has is a configuration error, unless it is off", () => {
    assert.throws(() => verify(template("<div />"), configFor({}, { "vue/html-indent": 2 })), (error: unknown) => {
        assert.ok(error instanceof ConfigurationError);
        assert.equal(error.message, "No plugin has a rule named vue/html-indent");
        return true;
    });
    assert.deepEqual(verify(template("<div />"), configFor({}, { "vue/html-indent": 0 })), []);
});

test("what a rule throws says which rule, on which file", () => {
    const rule: HostedRule = {
        create(context) {
            if (context.options.length > 0) {
                throw new Error("the option unknown is not supported");
            }
            return {};
        },
    };

    assert.throws(
        () => verify(template("<div />"), configFor({ "test/strict": rule }, { "test/strict": [2, { unknown: true }] })),
        { message: `test/strict on ${FILE}: the option unknown is not supported` },
    );
});

test("a comment disables no rule", () => {
    const messages = verify(
        template('<!-- eslint-disable -->\n  <div v-html="html"></div>'),
        configFor({ "vue/no-v-html": noVHtml }),
    );

    assert.deepEqual(messages.map(message => message.ruleId), ["vue/no-v-html"]);
});

test("a syntax error in a script is the only report of a file", () => {
    const [message, ...others] = verify(
        '<script setup lang="ts">\nconst = 1\n</script>\n\n<template>\n  <div v-html="x" />\n</template>\n',
        configFor({ "vue/no-v-html": noVHtml }),
    );

    assert.deepEqual(others, []);
    assert.equal(message?.fatal, true);
    assert.equal(message?.ruleId, null);
    assert.equal(message?.line, 2);
    assert.match(message?.message ?? "", /^Parsing error: /u);
});

test("a rule reads the tokens and the comments of a script, and the scope of a node", () => {
    const rule: HostedRule = {
        create: context => ({
            VariableDeclaration(node: AstNode) {
                const { sourceCode } = context;
                const first = sourceCode.getFirstToken(node);
                const last = sourceCode.getLastToken(node);
                const after = first === null ? null : sourceCode.getTokenAfter(first, { includeComments: true });
                context.report({
                    message: [
                        first?.value,
                        after?.type,
                        last?.value,
                        sourceCode.getCommentsBefore(node).map(comment => comment.value.trim()),
                        sourceCode.getDeclaredVariables(node).map(variable => variable.name),
                        sourceCode.getScope(node)?.type,
                        sourceCode.getLocFromIndex(sourceCode.getIndexFromLoc(node.loc.start)).line,
                    ].join(" | "),
                    node,
                });
            },
        }),
    };

    assert.deepEqual(
        format(verify(
            '<script setup lang="ts">\n// about a\nconst /* inline */ a = 1\n</script>\n',
            configFor({ "test/tokens": rule }),
        )),
        ["3:1 const | Block | 1 | about a | a | module | 3"],
    );
});

/** A parser of custom blocks: every `"key":` of the block is a `Key` node, located in the text of the block. */
const keyParser = {
    parseForESLint(code: string) {
        const lines = code.split("\n");
        const locate = (offset: number) => {
            const before = code.slice(0, offset).split("\n");
            return { column: before.at(-1)!.length, line: before.length };
        };
        const node = (type: string, start: number, end: number) => ({
            loc: { end: locate(end), start: locate(start) },
            range: [start, end],
            type,
        });
        const keys = [...code.matchAll(/"(?<name>\w+)":/gu)].map(match => ({
            ...node("Key", match.index, match.index + match[0].length - 1),
            name: match.groups!["name"],
        }));

        return {
            ast: { ...node("Program", 0, code.length), body: keys, comments: [], lines: lines.length, tokens: [] },
            visitorKeys: { Key: [], Program: ["body"] },
        };
    },
};

test("a rule lints a custom block with its own parser, at the place of the block in the file", () => {
    const rule: HostedRule = {
        create: context =>
            context.sourceCode.parserServices.defineCustomBlocksVisitor(context, keyParser, {
                create: blockContext => ({
                    Key(node: AstNode) {
                        if (node["name"] === "unused") {
                            blockContext.report({
                                fix: fixer => fixer.replaceText(node, '"used"'),
                                message: `unused key in <${String((blockContext.parserServices["customBlock"] as AstNode)["name"])}>`,
                                node,
                            });
                        }
                    },
                }),
                target: lang => lang === "json",
            }),
        meta: { fixable: "code" },
    };
    const source = '<template>\n  <div />\n</template>\n\n<i18n lang="json">\n{\n  "hello": "Hello",\n  "unused": "Bye"\n}\n</i18n>\n\n<i18n lang="yaml">\n"unused": no\n</i18n>\n';
    const config = configFor({ "test/keys": rule });

    assert.deepEqual(format(verify(source, config)), ["8:3 unused key in <i18n>"]);
    assert.equal(verifyAndFix(source, config).output, source.replace('"unused": "Bye"', '"used": "Bye"'));
});
