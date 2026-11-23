//! `js/no-invalid-this`: reports `this` where it is `undefined`.
//!
//! Port of `src/rules/core/no-invalid-this.ts` of revolugo-lint-plugins, itself a port of the `no-invalid-this` rule
//! of `ESLint` (MIT license) that knows the `this` parameter and the class members of TypeScript. `this` is
//! `undefined` at the top level of a module, and in a strict function that nothing shows to be called with a `this`:
//! not a method, not a constructor (a capitalized name, unless `capIsConstructor` is false), without a `this`
//! parameter or a `@this` comment, and neither bound nor given a `this` argument where it is written
//! (`.call(obj)`, `array.map(fn, obj)`).
//!
//! The JS rule learns from the code paths of `ESLint` where a function, a class field initializer, a static block
//! and the program start and end. Here each `this` walks up to the first of them instead: the rule listens to
//! `this` alone, and a file without `this` costs nothing.
//!
//! The JS rule reads `ESTree`, where parentheses have no node and a method is a function expression under a
//! `MethodDefinition`. The comments name the `ESTree` node wherever the two trees differ.

use tsrs_ast::{self as ast, Kind, ModifierFlags, Node, NodeFlags, SourceFile};
use tsrs_core::P;
use tsrs_core::json::Value;

use crate::protocol::RuleMessage;
use crate::rule::{Listener, Rule, RuleContext};
use crate::utils::is_str_white_space;

const NAME: &str = "js/no-invalid-this";

/// The array methods whose second argument is the `this` of their callback.
const ARRAY_METHODS_WITH_THIS_ARGUMENT: [&str; 10] = [
    "every",
    "filter",
    "find",
    "findIndex",
    "findLast",
    "findLastIndex",
    "flatMap",
    "forEach",
    "map",
    "some",
];

pub struct NoInvalidThis {
    cap_is_constructor: bool,
    /// How Oxlint reads the file. Found at the first `this` that depends on it: most are in methods and do not.
    file_mode: Option<FileMode>,
    /// The last function a `this` was found in, and whether `this` is valid there: a function often has several.
    last_function: Option<(P<Node>, bool)>,
}

/// What the top level of a file says about `this`.
#[derive(Clone, Copy)]
struct FileMode {
    /// False in a module and in a strict `CommonJS` file, where `this` is `undefined` at the top level.
    top_level_this: bool,
    /// True when the whole file is strict mode code.
    strict: bool,
}

impl NoInvalidThis {
    pub fn new(options: &[Value]) -> Result<Self, String> {
        let cap_is_constructor = match options {
            [] => true,
            [Value::Object(option)] => {
                let mut cap_is_constructor = true;
                for (key, value) in option {
                    match (key, value) {
                        ("capIsConstructor", Value::Bool(value)) => cap_is_constructor = *value,
                        ("capIsConstructor", _) => {
                            return Err(format!(
                                "{NAME}: the option `capIsConstructor` is a boolean"
                            ));
                        }
                        _ => return Err(format!("{NAME}: unknown option `{key}`")),
                    }
                }
                cap_is_constructor
            }
            [_] => {
                return Err(format!("{NAME}: the options are an object, `{{ capIsConstructor }}`"));
            }
            _ => return Err(format!("{NAME}: takes one options object, `{{ capIsConstructor }}`")),
        };
        Ok(Self { cap_is_constructor, file_mode: None, last_function: None })
    }

    /// Whether `this` has a value where it is written: finds what `ESLint` calls its code path.
    fn is_valid(&mut self, this: P<Node>, file: P<SourceFile>) -> bool {
        let mut child = this;
        while let Some(parent) = child.parent() {
            match parent.kind() {
                // A function without a body is an overload or a `declare function`: no code path.
                Kind::FunctionDeclaration | Kind::FunctionExpression if parent.body().is_some() => {
                    return self.is_valid_in_function(parent, file);
                }
                // The function of a method is the value of a `MethodDefinition` or of a `Property`: `this` is
                // always valid in it. Its key and its decorators are outside of it.
                Kind::MethodDeclaration
                | Kind::Constructor
                | Kind::GetAccessor
                | Kind::SetAccessor
                    if parent.body().is_some()
                        && child.kind() != Kind::Decorator
                        && parent.name() != Some(child) =>
                {
                    return true;
                }
                // A class field initializer, and the value of an accessor property.
                Kind::PropertyDeclaration if parent.initializer() == Some(child) => return true,
                Kind::ClassStaticBlockDeclaration if child.kind() == Kind::Block => return true,
                // An arrow function has the `this` of the code around it.
                _ => {}
            }
            child = parent;
        }
        self.file_mode(file).top_level_this
    }

    fn is_valid_in_function(&mut self, function: P<Node>, file: P<SourceFile>) -> bool {
        if let Some((last, valid)) = self.last_function
            && last == function
        {
            return valid;
        }
        // `this` of a non-strict function is the global object at worst.
        let valid = !self.is_strict(function, file) || !self.has_default_this(function, file);
        self.last_function = Some((function, valid));
        valid
    }

    fn file_mode(&mut self, file: P<SourceFile>) -> FileMode {
        *self.file_mode.get_or_insert_with(|| file_mode(file))
    }

    /// Whether `function` is strict mode code, as typescript-eslint's scope manager says in Oxlint.
    fn is_strict(&mut self, function: P<Node>, file: P<SourceFile>) -> bool {
        if self.file_mode(file).strict || has_use_strict_directive(function, file.text()) {
            return true;
        }
        let mut child = function;
        while let Some(parent) = child.parent() {
            let strict = match parent.kind() {
                // The decorators of a class are outside of its scope.
                Kind::ClassDeclaration | Kind::ClassExpression => child.kind() != Kind::Decorator,
                Kind::ModuleDeclaration | Kind::EnumDeclaration => true,
                _ => has_use_strict_directive(parent, file.text()),
            };
            if strict {
                return true;
            }
            child = parent;
        }
        false
    }

    /// True for a function whose `this` is `undefined` when it is strict.
    fn has_default_this(&self, function: P<Node>, file: P<SourceFile>) -> bool {
        let is_constructor = self.cap_is_constructor
            && function.name().is_some_and(|name| starts_with_upper_case(name.text()));
        !is_constructor
            && !has_this_parameter(function)
            && self.is_unbound(function)
            && !has_this_tag(function, file.text())
    }

    /// Walks up from `function` towards what gives it a `this`: false when something does.
    fn is_unbound(&self, function: P<Node>) -> bool {
        let mut current = outermost(function);
        // False once the walk has left the function for an expression that evaluates to it.
        let mut direct = true;
        loop {
            let Some(parent) = current.parent() else { return true };
            let next = match parent.kind() {
                // The expressions that evaluate to one of their operands, unchanged. The JS rule does not ask
                // which operand `current` is.
                Kind::AsExpression
                | Kind::NonNullExpression
                | Kind::SatisfiesExpression
                | Kind::TypeAssertionExpression
                | Kind::ConditionalExpression => parent,
                Kind::ExpressionWithTypeArguments => {
                    if is_heritage(parent) {
                        return true;
                    }
                    parent
                }
                Kind::BinaryExpression => {
                    let binary = parent.as_binary_expression();
                    let operator = binary.operator_token.kind();
                    // `AssignmentExpression`, or `AssignmentPattern` in a destructuring assignment.
                    if ast::is_assignment_operator(operator) {
                        return self
                            .is_assigned_without_this(function, skip_assertions(binary.left));
                    }
                    // `LogicalExpression`.
                    if !matches!(
                        operator,
                        Kind::BarBarToken
                            | Kind::AmpersandAmpersandToken
                            | Kind::QuestionQuestionToken
                    ) {
                        return true;
                    }
                    parent
                }
                // `(() => function () {})()`
                Kind::ArrowFunction => {
                    if parent.body() != Some(current) {
                        return true;
                    }
                    let Some(call) = immediate_call(parent) else { return true };
                    call
                }
                Kind::CallExpression => return is_called_without_this(current, parent),
                Kind::PropertyAccessExpression | Kind::ElementAccessExpression => {
                    return is_member_without_this(current, parent);
                }
                // The value of a `Property`.
                Kind::PropertyAssignment => {
                    return parent.as_property_assignment().initializer.get() != current;
                }
                // The value of a `PropertyDefinition`. A function that is the value of an `AccessorProperty`
                // is valid as well, and one that the value evaluates to is not.
                Kind::PropertyDeclaration => {
                    return parent.initializer() != Some(current)
                        || (ast::has_accessor_modifier(parent) && !direct);
                }
                // `(function () { return function () {} })()`
                Kind::ReturnStatement => {
                    let Some(call) = upper_function(parent).and_then(immediate_call) else {
                        return true;
                    };
                    call
                }
                // `VariableDeclarator`, or `AssignmentPattern` for a default value.
                Kind::VariableDeclaration | Kind::Parameter | Kind::BindingElement => {
                    return parent.initializer() != Some(current)
                        || parent
                            .name()
                            .is_none_or(|name| self.is_assigned_without_this(function, name));
                }
                // `({ name = function () {} } = object)`: an `AssignmentPattern` as well.
                Kind::ShorthandPropertyAssignment => {
                    let property = parent.as_shorthand_property_assignment();
                    return property.object_assignment_initializer.get() != Some(current)
                        || self.is_assigned_without_this(function, property.name());
                }
                _ => return true,
            };
            direct = false;
            current = outermost(next);
        }
    }

    /// True unless the function is assigned to a member or to a capitalized name.
    fn is_assigned_without_this(&self, function: P<Node>, left: P<Node>) -> bool {
        match left.kind() {
            Kind::PropertyAccessExpression | Kind::ElementAccessExpression => false,
            Kind::Identifier => {
                !(self.cap_is_constructor
                    && function.name().is_none()
                    && starts_with_upper_case(left.text()))
            }
            _ => true,
        }
    }
}

impl Rule for NoInvalidThis {
    fn listens_to(&self, listener: Listener) -> bool {
        matches!(listener, Listener::Enter(Kind::ThisKeyword | Kind::TypeQuery))
    }

    fn visit(&mut self, _: Listener, node: P<Node>, context: &mut RuleContext<'_>) {
        let Some(this) = this_expression(node) else { return };
        if !self.is_valid(this, context.source_file) {
            context.report_node(
                this,
                RuleMessage {
                    id: "unexpectedThis".into(),
                    description: "Unexpected 'this'.".into(),
                    ..Default::default()
                },
            );
        }
    }
}

/// The `ThisExpression` of `ESTree` that `node` is or has: `this`, and the `this` of `typeof this.name` in a type.
fn this_expression(node: P<Node>) -> Option<P<Node>> {
    if node.kind() == Kind::ThisKeyword {
        return (!is_jsx_tag_name(node)).then_some(node);
    }
    // A type that the parser made from a JSDoc comment is not in ESTree.
    if node.flags().intersects(NodeFlags::Reparsed | NodeFlags::JSDoc) {
        return None;
    }
    let mut name = node.as_type_query_node().expr_name;
    while name.kind() == Kind::QualifiedName {
        name = name.as_qualified_name().left;
    }
    ast::is_this_identifier(name).then_some(name)
}

/// True for the `this` of `<this.Component />`, a `JSXIdentifier` in `ESTree`.
fn is_jsx_tag_name(this: P<Node>) -> bool {
    let mut node = this;
    while let Some(parent) = node.parent() {
        match parent.kind() {
            Kind::PropertyAccessExpression
                if parent.as_property_access_expression().expression == node =>
            {
                node = parent;
            }
            Kind::JsxOpeningElement | Kind::JsxSelfClosingElement | Kind::JsxClosingElement => {
                return true;
            }
            _ => return false,
        }
    }
    false
}

/// What Oxlint gives the JS rule for the file: its source type comes from the extension, and from the content for
/// the extensions that say neither module nor `CommonJS`.
fn file_mode(file: P<SourceFile>) -> FileMode {
    let strict = has_use_strict(file.statements().nodes(), file.text());
    let module = match file.file_name().rsplit_once('.').map(|(_, extension)| extension) {
        Some("mjs" | "mts") => true,
        // CommonJS: `this` is `module.exports` at the top level, unless the file is strict. The JS rule reports
        // it there, as ESLint does.
        Some("cjs" | "cts") => return FileMode { top_level_this: !strict, strict },
        _ => has_module_syntax(file),
    };
    // `this` at the top level of a script is the global object, strict or not.
    FileMode { top_level_this: !module, strict: module || strict }
}

/// True for a file that Oxlint's parser reads as a module when its extension does not say: one with an `import`
/// or an `export` statement, `import.meta` or an `await` outside of a function. `import name = require()` is not
/// one of them.
fn has_module_syntax(file: P<SourceFile>) -> bool {
    file.statements().nodes().iter().any(|&statement| {
        !statement.flags().intersects(NodeFlags::Reparsed)
            && (matches!(
                statement.kind(),
                Kind::ImportDeclaration
                    | Kind::ExportDeclaration
                    | Kind::ExportAssignment
                    | Kind::NamespaceExportDeclaration
            ) || ast::has_syntactic_modifier(statement, ModifierFlags::Export))
    }) || file.as_node().flags().intersects(NodeFlags::PossiblyContainsImportMeta)
        || (file.text().contains("await") && has_await_outside_functions(file.as_node()))
}

/// True when `node` has an `await` that no function of the file is around: Oxlint's parser takes it for the
/// `await` of a module. Only a file with the word `await` and no other module syntax is walked.
fn has_await_outside_functions(node: P<Node>) -> bool {
    node.for_each_child(&mut |child| match child.kind() {
        Kind::AwaitExpression => true,
        Kind::ForOfStatement if child.as_for_in_or_of_statement().await_modifier.is_some() => true,
        // `await using name = value`
        Kind::VariableDeclarationList
            if child.flags() & NodeFlags::BlockScoped == NodeFlags::AwaitUsing =>
        {
            true
        }
        Kind::FunctionDeclaration
        | Kind::FunctionExpression
        | Kind::ArrowFunction
        | Kind::MethodDeclaration
        | Kind::Constructor
        | Kind::GetAccessor
        | Kind::SetAccessor
        | Kind::InterfaceDeclaration
        | Kind::TypeAliasDeclaration => false,
        _ => has_await_outside_functions(child),
    })
}

/// True for a function whose body starts with a `'use strict'` directive.
fn has_use_strict_directive(function: P<Node>, text: &str) -> bool {
    matches!(
        function.kind(),
        Kind::FunctionDeclaration
            | Kind::FunctionExpression
            | Kind::ArrowFunction
            | Kind::MethodDeclaration
            | Kind::Constructor
            | Kind::GetAccessor
            | Kind::SetAccessor
    ) && function
        .body()
        .is_some_and(|body| body.kind() == Kind::Block && has_use_strict(body.statements(), text))
}

/// True when the directives that `statements` start with have `'use strict'`, written without an escape.
fn has_use_strict(statements: &[P<Node>], text: &str) -> bool {
    for &statement in statements {
        if statement.kind() != Kind::ExpressionStatement {
            return false;
        }
        let literal = statement.as_expression_statement().expression;
        if literal.kind() != Kind::StringLiteral {
            return false;
        }
        // The text of the literal is its value: the source says whether it is written as such.
        let end = position(literal.end());
        if literal.text() == "use strict"
            && end >= 12
            && text.as_bytes().get(end - 11..end - 1) == Some(b"use strict")
            && matches!(text.as_bytes()[end - 12], b'\'' | b'"')
        {
            return true;
        }
    }
    false
}

fn position(pos: tsrs_core::TextPos) -> usize {
    pos.expect_source("parsed lint node") as usize
}

/// True for `function (this: Type) {}`. In a JS file the parser makes this parameter from a `@this` comment as
/// well: the JS rule reads the comment instead.
fn has_this_parameter(function: P<Node>) -> bool {
    function.parameters().first().is_some_and(|&parameter| {
        ast::is_this_parameter(parameter) && !parameter.flags().intersects(NodeFlags::Reparsed)
    })
}

fn starts_with_upper_case(name: &str) -> bool {
    name.chars().next().is_some_and(|first| !first.to_lowercase().eq(std::iter::once(first)))
}

/// True for the nodes that `ESTree` does not have: parentheses, and the type assertion that the parser makes from
/// `/** @type {Type} */ (value)` in a JS file.
fn is_not_in_estree(node: P<Node>) -> bool {
    match node.kind() {
        Kind::ParenthesizedExpression => true,
        Kind::AsExpression | Kind::SatisfiesExpression => node
            .type_node()
            .is_some_and(|type_node| type_node.flags().intersects(NodeFlags::Reparsed)),
        _ => false,
    }
}

/// `node`, or the outermost parentheses around it: what the parent of the `ESTree` node has for child.
fn outermost(mut node: P<Node>) -> P<Node> {
    while let Some(parent) = node.parent()
        && is_not_in_estree(parent)
    {
        node = parent;
    }
    node
}

fn skip_parentheses(mut node: P<Node>) -> P<Node> {
    while is_not_in_estree(node)
        && let Some(expression) = node.expression()
    {
        node = expression;
    }
    node
}

fn is_assertion(node: P<Node>) -> bool {
    matches!(
        node.kind(),
        Kind::ParenthesizedExpression
            | Kind::AsExpression
            | Kind::NonNullExpression
            | Kind::SatisfiesExpression
            | Kind::TypeAssertionExpression
    )
}

/// The expression under `as`, `satisfies`, `<T>` and `!`.
fn skip_assertions(mut node: P<Node>) -> P<Node> {
    while is_assertion(node)
        && let Some(expression) = node.expression()
    {
        node = expression;
    }
    node
}

/// True for the expression of `class extends expression<T>`, which is not a `TSInstantiationExpression`.
fn is_heritage(expression_with_type_arguments: P<Node>) -> bool {
    expression_with_type_arguments
        .parent()
        .is_some_and(|parent| parent.kind() == Kind::HeritageClause)
}

/// The call of `function`, when it is called where it is written: `(function () {})()`.
fn immediate_call(function: P<Node>) -> Option<P<Node>> {
    let mut callee = function;
    while let Some(parent) = callee.parent()
        && (is_assertion(parent)
            || (parent.kind() == Kind::ExpressionWithTypeArguments && !is_heritage(parent)))
    {
        callee = parent;
    }
    let call = callee.parent()?;
    (call.kind() == Kind::CallExpression && call.as_call_expression().expression == callee)
        .then_some(call)
}

/// The function that `statement` returns from, when it is one that can be called where it is written.
fn upper_function(statement: P<Node>) -> Option<P<Node>> {
    let mut node = statement;
    while let Some(parent) = node.parent() {
        match parent.kind() {
            Kind::FunctionExpression | Kind::ArrowFunction => return Some(parent),
            Kind::FunctionDeclaration
            | Kind::MethodDeclaration
            | Kind::Constructor
            | Kind::GetAccessor
            | Kind::SetAccessor => return None,
            _ => node = parent,
        }
    }
    None
}

/// The object and the name of a member access whose name can be read without evaluating: `a.b`, `a['b']`.
fn member_access(node: P<Node>) -> Option<(P<Node>, &'static str)> {
    match node.kind() {
        Kind::PropertyAccessExpression => {
            let access = node.as_property_access_expression();
            (access.name.kind() == Kind::Identifier)
                .then(|| (access.expression, access.name.text()))
        }
        Kind::ElementAccessExpression => {
            let access = node.as_element_access_expression();
            let name = skip_parentheses(access.argument_expression);
            matches!(name.kind(), Kind::StringLiteral | Kind::NoSubstitutionTemplateLiteral)
                .then(|| (access.expression, name.text()))
        }
        _ => None,
    }
}

/// True for a `this` argument that leaves `this` undefined.
fn is_null_or_undefined(argument: P<Node>) -> bool {
    let node = skip_assertions(argument);
    match node.kind() {
        Kind::NullKeyword | Kind::VoidExpression => true,
        Kind::Identifier => node.text() == "undefined",
        _ => false,
    }
}

/// True when the call that `current` is a child of leaves `this` undefined: all calls but
/// `Reflect.apply(fn, obj, args)`, `Array.from(items, fn, obj)` and `array.map(fn, obj)` and the like.
fn is_called_without_this(current: P<Node>, call: P<Node>) -> bool {
    let call = call.as_call_expression();
    let arguments = call.arguments.nodes();
    let gives_this = |count: usize, position: usize| {
        arguments.len() == count
            && arguments[position] == current
            && !is_null_or_undefined(arguments[position + 1])
    };
    let Some((object, name)) = member_access(skip_parentheses(call.expression)) else {
        return true;
    };
    let object = skip_parentheses(object);
    let object = (object.kind() == Kind::Identifier).then(|| object.text());
    if name == "apply" && object == Some("Reflect") {
        return !gives_this(3, 0);
    }
    if (name == "from" && object.is_some_and(|object| object.ends_with("Array")))
        || (name == "fromAsync" && object == Some("Array"))
    {
        return !gives_this(3, 1);
    }
    if ARRAY_METHODS_WITH_THIS_ARGUMENT.contains(&name) {
        return !gives_this(2, 0);
    }
    true
}

/// True unless `member` is `fn.bind(obj)`, `fn.call(obj)` or `fn.apply(obj)`.
fn is_member_without_this(current: P<Node>, member: P<Node>) -> bool {
    let Some((object, name)) = member_access(member) else { return true };
    if object != current || !matches!(name, "bind" | "call" | "apply") {
        return true;
    }
    let callee = outermost(member);
    let Some(call) = callee.parent().filter(|parent| parent.kind() == Kind::CallExpression) else {
        return true;
    };
    let call = call.as_call_expression();
    !(call.expression == callee
        && call.arguments.nodes().first().is_some_and(|&argument| !is_null_or_undefined(argument)))
}

/// One comment, without its delimiters.
struct Comment<'a> {
    value: &'a str,
    block: bool,
    /// Where the comment ends in the text of the file.
    end: usize,
}

/// The comments between `pos`, where a token ends, and the next token.
struct Comments<'a> {
    text: &'a str,
    pos: usize,
}

impl<'a> Iterator for Comments<'a> {
    type Item = Comment<'a>;

    fn next(&mut self) -> Option<Comment<'a>> {
        loop {
            let rest = self.text.get(self.pos..)?;
            if let Some(line) = rest.strip_prefix("//") {
                let value = &line[..line.find(is_line_break).unwrap_or(line.len())];
                self.pos += 2 + value.len();
                return Some(Comment { value, block: false, end: self.pos });
            }
            if let Some(block) = rest.strip_prefix("/*") {
                // A comment that does not end runs to the end of the file.
                let (value, length) = match block.find("*/") {
                    Some(end) => (&block[..end], end + 4),
                    None => (block, rest.len()),
                };
                self.pos += length;
                return Some(Comment { value, block: true, end: self.pos });
            }
            // A hashbang is not a comment for Oxlint.
            if self.pos == 0 && rest.starts_with("#!") {
                self.pos = rest.find(is_line_break).unwrap_or(rest.len());
                continue;
            }
            let space = rest.chars().next().filter(|&character| is_str_white_space(character))?;
            self.pos += space.len_utf8();
        }
    }
}

fn is_line_break(character: char) -> bool {
    matches!(character, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn comments_before(text: &str, pos: usize) -> Comments<'_> {
    Comments { text, pos }
}

/// `/^[\s*]*@this/mu`: `@this` after nothing but spaces and asterisks on its line, or on the lines before it.
fn has_this_tag_in(comment: &str) -> bool {
    comment.match_indices("@this").any(|(index, _)| {
        let before = &comment[..index];
        let text =
            before.trim_end_matches(|character| character == '*' || is_str_white_space(character));
        text.is_empty() || before[text.len()..].contains(is_line_break)
    })
}

/// True when the token or the comment before the node that starts after `pos` is a `JSDoc` comment with `@this`
/// that ends on the line of the node or on the line above.
fn has_jsdoc_this_tag_above(text: &str, pos: usize) -> bool {
    let Some(comment) = comments_before(text, pos).last() else { return false };
    if !comment.block || !comment.value.starts_with('*') || !has_this_tag_in(comment.value) {
        return false;
    }
    let mut lines = 0;
    let mut previous = ' ';
    for character in
        text[comment.end..].chars().take_while(|&character| is_str_white_space(character))
    {
        if is_line_break(character) && !(character == '\n' && previous == '\r') {
            lines += 1;
        }
        previous = character;
    }
    lines <= 1
}

/// True when a comment says what the `this` of the function is.
fn has_this_tag(function: P<Node>, text: &str) -> bool {
    let (documented, start) =
        if function.kind() == Kind::FunctionDeclaration {
            // `export function` is a `FunctionDeclaration` in an `ExportNamedDeclaration`, which has the JSDoc.
            match function.modifier_nodes().iter().rfind(|modifier| {
                matches!(modifier.kind(), Kind::ExportKeyword | Kind::DefaultKeyword)
            }) {
                Some(export) => (Some(position(function.pos())), position(export.end())),
                None => (None, position(function.pos())),
            }
        } else {
            (documented_pos(function, text), position(function.pos()))
        };
    documented.is_some_and(|pos| has_jsdoc_this_tag_above(text, pos))
        // A callback has no JSDoc: `test(/* @this Sandbox */ function () {})`.
        || comments_before(text, start).any(|comment| has_this_tag_in(comment.value))
}

/// Where the node that carries the `JSDoc` comment of a function expression starts: the closest ancestor with a
/// comment before it, a function, a property or a method. None when it is the function itself.
fn documented_pos(function: P<Node>, text: &str) -> Option<usize> {
    let mut child = outermost(function);
    let mut parent = child.parent()?;
    if matches!(parent.kind(), Kind::CallExpression | Kind::NewExpression) {
        return None;
    }
    // Where the last ancestor starts: several start at the same token.
    let mut scanned = None;
    loop {
        match parent.kind() {
            Kind::FunctionDeclaration => return None,
            Kind::FunctionExpression
            | Kind::ArrowFunction
            | Kind::PropertyAssignment
            | Kind::ShorthandPropertyAssignment => return Some(position(parent.pos())),
            // The `MethodDefinition` or the `Property` from the key or a decorator, and from the rest its
            // value, a function that starts at the parameters: no JSDoc is there.
            Kind::MethodDeclaration | Kind::Constructor | Kind::GetAccessor | Kind::SetAccessor => {
                return (child.kind() == Kind::Decorator || parent.name() == Some(child))
                    .then(|| position(parent.pos()));
            }
            // `{ key: name = function () {} }`: an `AssignmentPattern` from `name` in a `Property` from `key`.
            Kind::BindingElement
                if parent
                    .parent()
                    .is_some_and(|pattern| pattern.kind() == Kind::ObjectBindingPattern) =>
            {
                let element = parent.as_binding_element();
                if element.property_name().is_some()
                    && let Some(name) = element.name()
                    && comments_before(text, position(name.pos())).next().is_some()
                {
                    return Some(position(name.pos()));
                }
                return Some(position(parent.pos()));
            }
            Kind::CaseBlock
            | Kind::HeritageClause
            | Kind::JsxAttributes
            | Kind::ComputedPropertyName => {}
            _ if is_not_in_estree(parent) => {}
            _ => {
                let pos = position(parent.pos());
                if scanned != Some(pos) {
                    if comments_before(text, pos).next().is_some() {
                        return Some(pos);
                    }
                    scanned = Some(pos);
                }
            }
        }
        child = parent;
        parent = child.parent()?;
    }
}
