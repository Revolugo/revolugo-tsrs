/**
 * How the rule host parses a .vue file: vue-eslint-parser for the file and
 * its `<template>`, @typescript-eslint/parser for its `<script>` blocks and
 * its template expressions. Both come from the linted project, and neither
 * needs ESLint.
 */
import { requireFromProject } from "../project.ts";
import { isRecord } from "./nodes.ts";
import type { Parsed } from "./sourceCode.ts";

/** A parser with ESLint's custom parser interface. */
interface Parser {
    parseForESLint: (code: string, options: object) => unknown;
}

function isParser(value: unknown): value is Parser {
    return isRecord(value) && typeof value["parseForESLint"] === "function";
}

/** The parser `name` of the project at `root`: it is loaded untyped. */
function parserOf(root: string, name: string, purpose: string): Parser {
    const loaded = requireFromProject(root, name, purpose);
    if (!isParser(loaded)) {
        throw new TypeError(`${name} is not the parser that the Vue lint expects`);
    }

    return loaded;
}

/** The parser options that ESLint's `Linter` gives, with our own. */
const parserOptions = (root: string): object => ({
    comment: true,
    ecmaVersion: "latest",
    eslintScopeManager: true,
    eslintVisitorKeys: true,
    loc: true,
    parser: parserOf(root, "@typescript-eslint/parser", "parses the <script> blocks and the template expressions"),
    range: true,
    sourceType: "module",
    tokens: true,
});

interface VueParser {
    options: object;
    parser: Parser;
}

/** The parsers of each project, by its root. */
const parsers = new Map<string, VueParser>();

function vueParserOf(root: string): VueParser {
    let vueParser = parsers.get(root);
    if (vueParser === undefined) {
        const parser = parserOf(root, "vue-eslint-parser", "parses the .vue files");
        vueParser = { options: parserOptions(root), parser };
        parsers.set(root, vueParser);
    }

    return vueParser;
}

interface ParseResult {
    ast: Parsed["ast"];
    scopeManager?: Parsed["scopeManager"];
    services: Parsed["parserServices"];
    visitorKeys: Parsed["visitorKeys"];
}

const SERVICES = [
    "defineCustomBlocksVisitor",
    "defineDocumentVisitor",
    "defineTemplateBodyVisitor",
    "getDocumentFragment",
    "getTemplateBodyTokenStore",
];

function isParseResult(value: unknown): value is ParseResult {
    if (!isRecord(value)) {
        return false;
    }
    const { ast, services, visitorKeys } = value;

    return (
        isRecord(ast)
        && ast["type"] === "Program"
        && Array.isArray(ast["tokens"])
        && Array.isArray(ast["comments"])
        && isRecord(visitorKeys)
        && isRecord(services)
        && SERVICES.every(service => typeof services[service] === "function")
    );
}

/**
 * Parses the .vue file `text`, which is at `filePath`, with the parsers of
 * the project at `root`. A syntax error of a `<script>` block is thrown, as
 * the parser's `ParseError` (./host.ts reports it); one of the `<template>`
 * is in `ast.templateBody.errors`.
 */
export function parseVue(text: string, filePath: string, root: string): Parsed {
    const { options, parser } = vueParserOf(root);
    const result = parser.parseForESLint(text, { ...options, filePath });
    if (!isParseResult(result)) {
        throw new TypeError(`vue-eslint-parser did not return a tree with its services for ${filePath}`);
    }

    return {
        ast: result.ast,
        parserServices: result.services,
        scopeManager: result.scopeManager ?? null,
        visitorKeys: result.visitorKeys,
    };
}
