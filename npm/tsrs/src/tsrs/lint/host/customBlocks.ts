/**
 * `parserServices.defineCustomBlocksVisitor` for the rule host: how a rule
 * lints the custom blocks of a .vue file, such as an `<i18n>` block that
 * holds its messages (the rules of @intlify/eslint-plugin-vue-i18n do).
 * vue-eslint-parser has
 * this service, but builds the block's source code with ESLint's
 * `SourceCode` class: this one does the same with ours.
 *
 * A block is parsed by the parser of the rule (JSON, YAML), its tree is
 * moved to where the block is in the file, and the listeners of the rule
 * are called on it, by node type. A block with a `src` attribute holds
 * nothing to lint.
 */
import { isRecord } from "./nodes.ts";
import {
    createSourceCode,
    lineOf,
} from "./sourceCode.ts";
import type {
    AstNode,
    CustomBlockParser,
    CustomBlockRule,
    Position,
    Program,
    Range,
    RuleContext,
    ScopeManager,
    ScriptVisitor,
    SourceCode,
    Token,
    VElement,
    VisitorKeys,
} from "./types.ts";
import {
    addListeners,
    childNodes,
    walk,
} from "./walk.ts";
import type { Listeners } from "./walk.ts";

/** A rule that lints custom blocks, with the context it was created with. */
interface Factory {
    context: RuleContext;
    rule: CustomBlockRule;
}

/** What a parser returns for the content of a block. */
interface ParsedBlock {
    ast: Program;
    scopeManager?: ScopeManager | null;
    services?: Record<string, unknown>;
    visitorKeys?: VisitorKeys;
}

const NOT_CUSTOM = new Set(["script", "style", "template"]);

function isElement(node: AstNode): node is VElement {
    return node.type === "VElement";
}

/** The blocks of the file that hold their content: not `<i18n src="...">`. */
const inlineCustomBlocks = (sourceCode: SourceCode): VElement[] =>
    (sourceCode.parserServices.getDocumentFragment()?.children ?? [])
        .filter(isElement)
        .filter(
            block =>
                !NOT_CUSTOM.has(block.name)
                && block.endTag !== null
                && !block.startTag.attributes.some(attribute => !attribute.directive && attribute.key.name === "src"),
        );

/** The `lang` of a block, `null` without one. */
function langOf(block: VElement): string | null {
    const lang = block.startTag.attributes.find(attribute => !attribute.directive && attribute.key.name === "lang");

    return lang?.directive === false && lang.value !== null && lang.value.value ? lang.value.value : null;
}

function isParsedBlock(value: unknown): value is ParsedBlock {
    if (!isRecord(value)) {
        return false;
    }
    const { ast } = value;

    return isRecord(ast) && ast["type"] === "Program" && Array.isArray(ast["tokens"]) && Array.isArray(ast["comments"]);
}

/** Where the content of a block is in the file. */
function contentRange(block: VElement): Range {
    const [text] = block.children;

    return text?.type === "VText" ? text.range : [block.startTag.range[1], block.endTag?.range[0] ?? block.range[1]];
}

const LINE_TERMINATOR = /\r\n|[\n\r]/gu;

/** Where an offset of a text is, as a line and a column. */
type Locate = (offset: number) => Position;

/**
 * How vue-eslint-parser locates an offset of the file, which is how the
 * nodes of the `<template>` are located: U+2028 and U+2029 end no line,
 * unlike in `sourceCode.getLocFromIndex`.
 */
function locatorOf(text: string): Locate {
    const starts = [0, ...[...text.matchAll(LINE_TERMINATOR)].map(match => match.index + match[0].length)];

    return offset => {
        const line = lineOf(starts, offset);

        return { column: offset - (starts[line] ?? 0), line: line + 1 };
    };
}

/** The tree of a block that its parser rejects: no content to lint. */
const emptyProgram = (locate: Locate, range: Range): ParsedBlock => ({
    ast: {
        body: [],
        comments: [],
        loc: { end: locate(range[1]), start: locate(range[0]) },
        range: [range[0], range[1]],
        sourceType: "module",
        tokens: [],
        type: "Program",
    },
});

/**
 * Moves the locations of a tree that was parsed from the content of a block
 * alone to where the content is in the file, `offset` characters further.
 */
function relocate(parsed: ParsedBlock, offset: number, locate: Locate): void {
    // A parser may give the same range or location object to two nodes.
    const moved = new Set<object>();
    const move = (target: AstNode | Token): void => {
        if (!moved.has(target.range)) {
            moved.add(target.range);
            target.range[0] += offset;
            target.range[1] += offset;
        }
        if (!moved.has(target.loc)) {
            moved.add(target.loc);
            target.loc.start = locate(target.range[0]);
            target.loc.end = locate(target.range[1]);
        }
    };
    const moveTree = (node: AstNode): void => {
        if (moved.has(node)) {
            return;
        }
        moved.add(node);
        move(node);
        for (const child of childNodes(node, parsed.visitorKeys ?? {})) {
            child.parent = node;
            moveTree(child);
        }
    };
    moveTree(parsed.ast);
    for (const token of [...parsed.ast.tokens, ...parsed.ast.comments]) {
        move(token);
    }
}

/** The tree of the content of a block, located in the file. */
function parseBlock(context: RuleContext, block: VElement, parser: CustomBlockParser): ParsedBlock {
    const { sourceCode } = context;
    const range = contentRange(block);
    const locate = locatorOf(sourceCode.text);
    try {
        const parsed = parser.parseForESLint(sourceCode.text.slice(range[0], range[1]), {
            comment: true,
            ecmaVersion: "latest",
            eslintScopeManager: true,
            eslintVisitorKeys: true,
            filePath: context.filename,
            loc: true,
            range: true,
            raw: true,
            sourceType: "module",
            tokens: true,
        });
        if (!isParsedBlock(parsed)) {
            throw new TypeError("the parser of a custom block returned no tree");
        }
        relocate(parsed, range[0], locate);

        return parsed;
    }
    catch (error) {
        if (!(error instanceof Error)) {
            throw error;
        }

        return {
            ...emptyProgram(locate, range),
            services: { parseError: error },
        };
    }
}

/** Lints a block with the rules that take it. */
function lintBlock(block: VElement, parser: CustomBlockParser, factories: readonly Factory[]): void {
    const lang = langOf(block);
    const [first, ...others] = factories.filter(({ rule }) => rule.target(lang, block));
    if (first === undefined) {
        return;
    }
    const file = first.context.sourceCode;
    const parsed = parseBlock(first.context, block, parser);
    const sourceCode = createSourceCode(
        file.text,
        {
            ast: parsed.ast,
            parserServices: {
                ...file.parserServices,
                ...parsed.services,
                customBlock: block,
            },
            scopeManager: parsed.scopeManager ?? null,
            visitorKeys: parsed.visitorKeys ?? {},
        },
        file.hasBOM,
    );
    const listeners: Listeners = new Map();
    for (const { context, rule } of [first, ...others]) {
        addListeners(
            listeners,
            context.id,
            rule.create({
                ...context,
                // What a rule reads from a context that is not ESLint's latest.
                getSourceCode: () => sourceCode,
                parserServices: sourceCode.parserServices,
                sourceCode,
            }),
        );
    }
    walk(parsed.ast, sourceCode.visitorKeys, listeners);
}

/** The rules of each file that lint its custom blocks, by the parser they give. */
const registries = new WeakMap<SourceCode, Map<unknown, Factory[]>>();

/**
 * Registers `rule` for the custom blocks of the file of `context`, which
 * `parser` parses. The first rule to come with a parser gets the listener
 * that lints the blocks, with every rule that came with that parser, when
 * the walk of the script leaves the Program.
 */
export function defineCustomBlocksVisitor(
    context: RuleContext,
    parser: CustomBlockParser,
    rule: CustomBlockRule,
): ScriptVisitor {
    const { sourceCode } = context;
    const blocks = inlineCustomBlocks(sourceCode);
    if (blocks.length === 0) {
        return {};
    }
    const registry = registries.get(sourceCode) ?? new Map<unknown, Factory[]>();
    registries.set(sourceCode, registry);
    const registered = registry.get(parser.parseForESLint);
    if (registered !== undefined) {
        registered.push({ context, rule });

        return {};
    }
    const factories = [{ context, rule }];
    registry.set(parser.parseForESLint, factories);

    return {
        "Program:exit"() {
            for (const block of blocks) {
                lintBlock(block, parser, factories);
            }
        },
    };
}
