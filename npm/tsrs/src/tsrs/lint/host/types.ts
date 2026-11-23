/**
 * The rule API of the rule host (./host.ts): what a hosted rule receives and
 * returns. It is the part of ESLint's rule API that the rules of a `.vue`
 * file use, those of eslint-plugin-vue and of @intlify/eslint-plugin-vue-i18n
 * among them.
 *
 * The template tree is vue-eslint-parser's. Its own types are not imported:
 * the parser comes from the linted project (./parser.ts), and this package
 * depends on nothing.
 */

export interface Position {
    column: number;
    line: number;
}

export interface SourceLocation {
    end: Position;
    start: Position;
}

export type Range = [number, number];

/** A node of the tree a rule reads: ESTree with TypeScript's nodes in a script, vue-eslint-parser's in a template. */
export interface AstNode {
    [key: string]: unknown;
    loc: SourceLocation;
    parent?: AstNode;
    range: Range;
    type: string;
}

export interface Token {
    loc: SourceLocation;
    range: Range;
    type: string;
    value: string;
}

export interface Fix {
    range: Range;
    text: string;
}

export interface Fixer {
    insertTextAfter: (target: Located, text: string) => Fix;
    insertTextAfterRange: (range: Range, text: string) => Fix;
    insertTextBefore: (target: Located, text: string) => Fix;
    insertTextBeforeRange: (range: Range, text: string) => Fix;
    remove: (target: Located) => Fix;
    removeRange: (range: Range) => Fix;
    replaceText: (target: Located, text: string) => Fix;
    replaceTextRange: (range: Range, text: string) => Fix;
}

export type Located = AstNode | Token;

/** A listener of the script tree, by node type (`:exit` when leaving the node). */
export type ScriptVisitor = Record<string, (node: AstNode) => void>;

/**
 * Listeners of the template tree, by esquery selector. vue-eslint-parser
 * calls them, with the node type that the selector matches.
 */
export type TemplateVisitor = Record<string, (node: never) => void>;

interface VIdentifier extends AstNode {
    name: string;
    rawName: string;
    type: "VIdentifier";
}

interface VLiteral extends AstNode {
    type: "VLiteral";
    value: string;
}

export interface VExpressionContainer extends AstNode {
    expression: AstNode | null;
    type: "VExpressionContainer";
}

export interface VDirectiveKey extends AstNode {
    argument: VExpressionContainer | VIdentifier | null;
    modifiers: VIdentifier[];
    name: VIdentifier;
    type: "VDirectiveKey";
}

/** A plain attribute: `class="a"`, `disabled`. */
export interface VPlainAttribute extends AstNode {
    directive: false;
    key: VIdentifier;
    parent: VStartTag;
    type: "VAttribute";
    value: VLiteral | null;
}

/** A directive: `v-if="a"`, `:class="a"`, `@click="a"`. */
export interface VDirective extends AstNode {
    directive: true;
    key: VDirectiveKey;
    parent: VStartTag;
    type: "VAttribute";
    value: VExpressionContainer | null;
}

export type VAttribute = VDirective | VPlainAttribute;

export interface VStartTag extends AstNode {
    attributes: VAttribute[];
    parent: VElement;
    type: "VStartTag";
}

interface VEndTag extends AstNode {
    type: "VEndTag";
}

export interface VElement extends AstNode {
    children: AstNode[];
    endTag: VEndTag | null;
    name: string;
    namespace: string;
    rawName: string;
    startTag: VStartTag;
    type: "VElement";
}

interface VDocumentFragment extends AstNode {
    children: AstNode[];
    comments: Token[];
    tokens: Token[];
    type: "VDocumentFragment";
}

/** The `<template>` of the file, as vue-eslint-parser hangs it on the Program. */
interface TemplateBody extends VElement {
    comments: Token[];
    errors: unknown[];
}

export interface Program extends AstNode {
    comments: Token[];
    templateBody?: TemplateBody;
    tokens: Token[];
    type: "Program";
}

/** The tokens of the whole file, `<template>` included. */
interface TemplateTokenStore {
    getFirstToken: (node: Located) => Token | null;
    getLastToken: (node: Located) => Token | null;
    getTokenAfter: (node: Located) => Token | null;
    getTokenBefore: (node: Located) => Token | null;
}

/** What a parser gives to `parserServices.defineCustomBlocksVisitor`. */
export interface CustomBlockParser {
    parseForESLint: (code: string, options: object) => unknown;
}

/** The context of a rule on a custom block: `sourceCode` is that of the block. */
interface CustomBlockContext extends RuleContext {
    parserServices: ParserServices;
    getSourceCode: () => SourceCode;
}

/** A rule for the custom blocks (`<i18n>`) that `target` accepts. */
export interface CustomBlockRule {
    create: (context: CustomBlockContext) => unknown;
    target: (lang: string | null, block: VElement) => boolean;
}

/** The services of vue-eslint-parser that the hosted rules use. */
export interface ParserServices {
    [key: string]: unknown;
    defineCustomBlocksVisitor: (
        context: RuleContext,
        parser: CustomBlockParser,
        rule: CustomBlockRule,
    ) => ScriptVisitor;
    defineDocumentVisitor: (documentVisitor: TemplateVisitor) => ScriptVisitor;
    defineTemplateBodyVisitor: (templateBodyVisitor: TemplateVisitor, scriptVisitor?: ScriptVisitor) => ScriptVisitor;
    getDocumentFragment: () => VDocumentFragment | null;
    getTemplateBodyTokenStore: () => TemplateTokenStore;
}

/** How a variable is declared: `ImportBinding`, `Variable`, `Parameter`... */
export interface Definition {
    node: AstNode;
    /** The declaration that holds `node`: the import of an import specifier. */
    parent?: AstNode | null;
    type: string;
}

export interface Variable {
    defs: Definition[];
    /** typescript-eslint's: whether the name can be used as a type. */
    isTypeVariable?: boolean;
    /** typescript-eslint's: whether the name can be used as a value. */
    isValueVariable?: boolean;
    name: string;
}

export interface Scope {
    childScopes: Scope[];
    set: Map<string, Variable>;
    type: string;
    upper: Scope | null;
    variables: Variable[];
}

/** The child properties of each node type, in the order they are visited. */
export type VisitorKeys = Record<string, readonly string[] | undefined>;

export interface ScopeManager {
    globalScope: Scope | null;
    scopes: Scope[];
    acquire: (node: AstNode, inner?: boolean) => Scope | null;
    getDeclaredVariables: (node: AstNode) => Variable[];
}

/** How a rule skips or filters tokens: ESLint's options, without `count`. */
export interface TokenOptions {
    includeComments?: boolean;
    skip?: number;
    filter?: (token: Token) => boolean;
}

/** What a rule passes to a token method: options, a count to skip, or a filter. */
export type TokenQuery = TokenOptions | number | ((token: Token) => boolean) | undefined;

/** The methods of ESLint's token store that the hosted rules call. */
export interface TokenStore {
    getAllComments: () => Token[];
    getCommentsAfter: (target: Located) => Token[];
    getCommentsBefore: (target: Located) => Token[];
    getCommentsInside: (node: Located) => Token[];
    getFirstToken: (node: Located, query?: TokenQuery) => Token | null;
    getLastToken: (node: Located, query?: TokenQuery) => Token | null;
    getTokenAfter: (target: Located, query?: TokenQuery) => Token | null;
    getTokenBefore: (target: Located, query?: TokenQuery) => Token | null;
    getTokens: (node: Located, query?: TokenQuery) => Token[];
}

/** The tree of a file or of a custom block, with its text and its tokens. */
export interface SourceCode extends TokenStore {
    ast: Program;
    hasBOM: boolean;
    lines: string[];
    parserServices: ParserServices;
    scopeManager: ScopeManager | null;
    text: string;
    visitorKeys: VisitorKeys;
    getDeclaredVariables: (node: AstNode) => Variable[];
    getIndexFromLoc: (position: Position) => number;
    getLoc: (target: Located) => SourceLocation;
    getLocFromIndex: (index: number) => Position;
    getRange: (target: Located) => Range;
    getScope: (node: AstNode) => Scope | null;
    getText: (target?: Located, before?: number, after?: number) => string;
}

export interface ReportDescriptor {
    /** The values of the `{{placeholders}}` of the message. */
    data?: Record<string, unknown>;
    /** One fix, or several that the host merges into one. */
    fix?: ((fixer: Fixer) => Fix | Iterable<Fix> | null | undefined) | null;
    loc?: Position | SourceLocation;
    message?: string;
    /** A key of the rule's `meta.messages`, instead of `message`. */
    messageId?: string;
    node?: Located;
}

export interface RuleContext {
    /** The name the rule is configured with: `vue/no-v-html`. */
    id: string;
    /** The root of the linted project. */
    cwd: string;
    /**
     * Every rule entry that the configuration resolves for the file, by the
     * name it has there, the rules that the host does not run included: a
     * rule reads what the script of the file is held to (`no-restricted-syntax`)
     * to hold the template to it too. Not in ESLint's rule API.
     */
    fileRules: ReadonlyMap<string, unknown>;
    filename: string;
    languageOptions: {
        ecmaVersion: "latest";
        parserOptions: Record<string, unknown>;
        sourceType: "module";
    };
    options: unknown[];
    physicalFilename: string;
    settings: Record<string, unknown>;
    sourceCode: SourceCode;
    report: (descriptor: ReportDescriptor) => void;
}

interface RuleMeta {
    fixable?: "code" | "whitespace" | null;
    messages?: Record<string, string>;
    type?: "layout" | "problem" | "suggestion";
}

/** A rule as the host runs it: what a plugin exports under `rules`. */
export interface HostedRule {
    meta?: RuleMeta;
    create: (context: RuleContext) => unknown;
}

export type Severity = 1 | 2;

export interface LintMessage {
    column: number;
    endColumn?: number;
    endLine?: number;
    /** A parsing error: no rule ran. */
    fatal?: true;
    fix?: Fix;
    line: number;
    message: string;
    messageId?: string;
    /** `null` for a parsing error. */
    ruleId: string | null;
    severity: Severity;
}
