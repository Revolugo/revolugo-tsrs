/**
 * The `sourceCode` of a rule context: the text of a file, its tree, and the
 * helpers of ESLint's `SourceCode` that the hosted rules call.
 */
import { createTokenStore } from "./tokens.ts";
import type {
    AstNode,
    Located,
    ParserServices,
    Program,
    Scope,
    ScopeManager,
    SourceCode,
    VisitorKeys,
} from "./types.ts";

/** What a parser returns for a text. */
export interface Parsed {
    ast: Program;
    parserServices: ParserServices;
    scopeManager: ScopeManager | null;
    visitorKeys: VisitorKeys;
}

const LINE_BREAK = /\r\n|[\n\r\u2028\u2029]/gu;

/** The offset at which each line of `text` starts. */
function lineStarts(text: string): number[] {
    const starts = [0];
    for (const match of text.matchAll(LINE_BREAK)) {
        starts.push(match.index + match[0].length);
    }

    return starts;
}

/** The index in `starts` of the line that holds `offset`: the last one that starts at it or before. */
export function lineOf(starts: readonly number[], offset: number): number {
    let line = starts.length - 1;
    while (line > 0 && (starts[line] ?? 0) > offset) {
        line -= 1;
    }

    return line;
}

/** The innermost scope of `node`, as ESLint's `SourceCode#getScope` finds it. */
function scopeOf(scopeManager: ScopeManager, node: AstNode): Scope | null {
    const inner = node.type !== "Program";
    for (let current: AstNode | undefined = node; current !== undefined; current = current.parent) {
        const scope = scopeManager.acquire(current, inner);
        if (scope !== null) {
            return scope.type === "function-expression-name" ? (scope.childScopes[0] ?? scope) : scope;
        }
    }

    return scopeManager.scopes[0] ?? null;
}

/**
 * The source code of `text` (without its byte order mark, which `hasBOM`
 * records) as `parsed` describes it.
 */
export function createSourceCode(text: string, parsed: Parsed, hasBOM = false): SourceCode {
    const { ast, parserServices, scopeManager, visitorKeys } = parsed;
    const starts = lineStarts(text);
    const lines = text.split(LINE_BREAK);

    return {
        ...createTokenStore(ast.tokens, ast.comments),
        ast,
        getDeclaredVariables: node => scopeManager?.getDeclaredVariables(node) ?? [],
        getIndexFromLoc({ column, line }) {
            const start = starts[line - 1];
            if (start === undefined) {
                throw new RangeError(
                    `Line number out of range (line ${line} requested, but only ${starts.length} lines present).`,
                );
            }

            return start + column;
        },
        getLoc: target => target.loc,
        getLocFromIndex(index) {
            if (index < 0 || index > text.length) {
                throw new RangeError(
                    `Index out of range (requested index ${index}, but source text has length ${text.length}).`,
                );
            }
            const line = lineOf(starts, index);

            return { column: index - (starts[line] ?? 0), line: line + 1 };
        },
        getRange: target => target.range,
        getScope: node => (scopeManager === null ? null : scopeOf(scopeManager, node)),
        getText(target?: Located, before = 0, after = 0) {
            return target === undefined
                ? text
                : text.slice(Math.max(target.range[0] - before, 0), target.range[1] + after);
        },
        hasBOM,
        lines,
        parserServices,
        scopeManager,
        text,
        visitorKeys,
    };
}
