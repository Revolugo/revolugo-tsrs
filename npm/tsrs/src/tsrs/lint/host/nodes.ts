/** Reading a tree whose node types are not declared: the script tree and the template expressions. */
import type { AstNode } from "./types.ts";

/** Any non-null object, readable key by key. */
export function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === "object" && value !== null;
}

export function isNode(value: unknown): value is AstNode {
    return isRecord(value) && typeof value["type"] === "string";
}

/** The node at `key` of `node`, if there is one. */
export function childOf(node: AstNode, key: string): AstNode | undefined {
    const value = node[key];

    return isNode(value) ? value : undefined;
}

/** The nodes of the list at `key` of `node`, without its holes. */
export function childrenOf(node: AstNode, key: string): AstNode[] {
    const value = node[key];

    return Array.isArray(value) ? value.filter(isNode) : [];
}

/** The string at `key` of `node`, if it is one. */
export function stringOf(node: AstNode, key: string): string | undefined {
    const value = node[key];

    return typeof value === "string" ? value : undefined;
}

/** The `value` of a Literal node as `String()` gives it: its text for a string, its source for the others. */
export function literalText(literal: AstNode): string {
    const { value } = literal;
    if (typeof value === "string") {
        return value;
    }

    return value === null
        || typeof value === "bigint"
        || typeof value === "boolean"
        || typeof value === "number"
        || value instanceof RegExp
        ? String(value)
        : "";
}

/** The text of a part of a template literal, as the template evaluates it. */
export function cookedOf(quasi: AstNode): string | undefined {
    const { value } = quasi;

    return isRecord(value) && typeof value["cooked"] === "string" ? value["cooked"] : undefined;
}
