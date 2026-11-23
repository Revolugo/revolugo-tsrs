/**
 * The listeners of the rules on a tree, and the walk that calls them. A rule
 * listens to a node type (`CallExpression`), or to the children of a node
 * type (`JSONArrayExpression > *`, which the rules of
 * @intlify/eslint-plugin-vue-i18n use on a block of messages), and adds
 * `:exit` to be called when the walk leaves the node. There is no selector
 * engine here: any other esquery selector is rejected.
 */
import {
    isNode,
    isRecord,
} from "./nodes.ts";
import type {
    AstNode,
    VisitorKeys,
} from "./types.ts";

type Listener = (node: AstNode) => void;

/** The listeners of every rule, by node type, in the order of the rules. */
export type Listeners = Map<string, Listener[]>;

function isListener(value: unknown): value is Listener {
    return typeof value === "function";
}

/** A node type or the children of one, and `:exit` to listen when the walk leaves the node. */
const SELECTOR = /^[$A-Z_a-z][\w$]*(?: > \*)?(?::exit)?$/u;

/** Adds the listeners that the `create` of the rule `id` returned. */
export function addListeners(listeners: Listeners, id: string, created: unknown): void {
    if (!isRecord(created)) {
        throw new TypeError(`The create() function for rule '${id}' did not return an object.`);
    }
    for (const [event, listener] of Object.entries(created)) {
        if (!SELECTOR.test(event)) {
            throw new Error(
                `Rule '${id}' listens to the selector "${event}" outside the template: the rule host calls such listeners by node type, or by parent type ("Parent > *"), only`,
            );
        }
        if (isListener(listener)) {
            listeners.set(event, [...(listeners.get(event) ?? []), listener]);
        }
        else if (listener !== undefined) {
            throw new TypeError(`The listener "${event}" of '${id}' is not a function`);
        }
    }
}

const NOT_CHILDREN = new Set(["leadingComments", "parent", "trailingComments"]);

/** The child properties of a node whose type has no visitor keys, as ESLint picks them. */
const fallbackKeys = (node: AstNode): string[] =>
    Object.keys(node).filter(key => !NOT_CHILDREN.has(key) && !key.startsWith("_"));

/** The children of a node, in the order of the visitor keys of its type. */
export const childNodes = (node: AstNode, keys: VisitorKeys): AstNode[] =>
    (keys[node.type] ?? fallbackKeys(node)).flatMap(key => {
        const value = node[key];

        return (Array.isArray(value) ? value : [value]).filter(isNode);
    });

/** A node of the walk, with the type of the node it is a child of. */
interface Visit {
    node: AstNode;
    /** `undefined` for the root. */
    parentType: string | undefined;
}

/** Calls the listeners of a node, for the selectors `type` and `Parent > *` with this suffix. */
function emit(listeners: Listeners, { node, parentType }: Visit, suffix: string): void {
    const byType = `${node.type}${suffix}`;
    const byParent = `${parentType} > *${suffix}`;
    // As esquery orders two selectors of the same specificity: alphabetically.
    const events = parentType !== undefined && listeners.has(byParent) ? [byType, byParent].sort() : [byType];
    for (const event of events) {
        for (const listener of listeners.get(event) ?? []) {
            listener(node);
        }
    }
}

/**
 * Walks the tree in source order and calls the listeners: those of a node,
 * then its children, then those of the node with `:exit`. Each node but the
 * root gets its `parent`.
 */
export function walk(root: AstNode, keys: VisitorKeys, listeners: Listeners): void {
    const visit = (current: Visit): void => {
        emit(listeners, current, "");
        for (const child of childNodes(current.node, keys)) {
            child.parent = current.node;
            visit({ node: child, parentType: current.node.type });
        }
        emit(listeners, current, ":exit");
    };
    visit({ node: root, parentType: undefined });
}
