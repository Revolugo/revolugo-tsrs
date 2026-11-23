/**
 * The token and comment access of a `SourceCode` (./sourceCode.ts): the
 * methods of ESLint's token store that a hosted rule calls, over the tokens
 * and comments of one tree.
 */
import type {
    Located,
    Token,
    TokenOptions,
    TokenQuery,
    TokenStore,
} from "./types.ts";

const COMMENTS = new Set(["Block", "Line", "Shebang"]);

const isComment = (token: Token): boolean => COMMENTS.has(token.type);

function optionsOf(query: TokenQuery): TokenOptions {
    if (typeof query === "number") {
        return { skip: query };
    }
    if (typeof query === "function") {
        return { filter: query };
    }

    return query ?? {};
}

/** The index of the first token that starts at `offset` or after it. */
function firstFrom(list: readonly Token[], offset: number): number {
    let low = 0;
    let high = list.length;
    while (low < high) {
        const middle = Math.floor((low + high) / 2);
        const token = list[middle];
        if (token !== undefined && token.range[0] < offset) {
            low = middle + 1;
        }
        else {
            high = middle;
        }
    }

    return low;
}

/** The index of the last token that ends at `offset` or before it, -1 without one. */
function lastUntil(list: readonly Token[], offset: number): number {
    let low = 0;
    let high = list.length;
    while (low < high) {
        const middle = Math.floor((low + high) / 2);
        const token = list[middle];
        if (token !== undefined && token.range[1] <= offset) {
            low = middle + 1;
        }
        else {
            high = middle;
        }
    }

    return low - 1;
}

interface Scan {
    /** Where the scan starts in `list`. */
    from: number;
    list: readonly Token[];
    /** 1 towards the end of the file, -1 towards its start. */
    step: -1 | 1;
}

/** The first token of the scan that the options keep and `within` accepts. */
function pick({ from, list, step }: Scan, options: TokenOptions, within: (token: Token) => boolean): Token | null {
    let skip = options.skip ?? 0;
    for (let index = from; index >= 0 && index < list.length; index += step) {
        const token = list[index];
        if (token === undefined || !within(token)) {
            return null;
        }
        if (options.filter?.(token) ?? true) {
            if (skip === 0) {
                return token;
            }
            skip -= 1;
        }
    }

    return null;
}

const anywhere = (): boolean => true;

/** The token store of a tree, from its tokens and comments (both in source order). */
export function createTokenStore(tokens: readonly Token[], comments: readonly Token[]): TokenStore {
    const all = [...tokens, ...comments].sort((a, b) => a.range[0] - b.range[0]);
    const listFor = (options: TokenOptions): readonly Token[] => (options.includeComments === true ? all : tokens);
    const inside =
        (node: Located) =>
        (token: Token): boolean =>
            token.range[0] >= node.range[0] && token.range[1] <= node.range[1];

    /** The comments that touch `target` on one side, with no token in between. */
    const adjacentComments = (target: Located, step: -1 | 1): Token[] => {
        const found: Token[] = [];
        const from = step === 1 ? firstFrom(all, target.range[1]) : lastUntil(all, target.range[0]);
        for (let index = from; index >= 0 && index < all.length; index += step) {
            const token = all[index];
            if (token === undefined || !isComment(token)) {
                break;
            }
            found.push(token);
        }

        return step === 1 ? found : found.reverse();
    };

    return {
        getAllComments: () => [...comments],
        getCommentsAfter: target => adjacentComments(target, 1),
        getCommentsBefore: target => adjacentComments(target, -1),
        getCommentsInside: node => comments.filter(inside(node)),
        getFirstToken(node, query) {
            const options = optionsOf(query);
            const list = listFor(options);

            return pick({ from: firstFrom(list, node.range[0]), list, step: 1 }, options, inside(node));
        },
        getLastToken(node, query) {
            const options = optionsOf(query);
            const list = listFor(options);

            return pick({ from: lastUntil(list, node.range[1]), list, step: -1 }, options, inside(node));
        },
        getTokenAfter(target, query) {
            const options = optionsOf(query);
            const list = listFor(options);

            return pick({ from: firstFrom(list, target.range[1]), list, step: 1 }, options, anywhere);
        },
        getTokenBefore(target, query) {
            const options = optionsOf(query);
            const list = listFor(options);

            return pick({ from: lastUntil(list, target.range[0]), list, step: -1 }, options, anywhere);
        },
        getTokens(node, query) {
            const options = optionsOf(query);

            return listFor(options)
                .filter(inside(node))
                .filter(token => options.filter?.(token) ?? true);
        },
    };
}
