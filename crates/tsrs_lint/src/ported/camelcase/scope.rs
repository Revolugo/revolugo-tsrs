//! The scopes of typescript-eslint's scope manager, which Oxlint gives a JS rule, read from the TypeScript AST.
//!
//! A scope is named by the node that opens it. The rule asks two questions: where a name is declared, and which
//! declarations an identifier can see. Both walk the ancestors of a node, so nothing is computed for a file that has
//! no underscored name.

use tsrs_ast::{Kind, ModifierFlags, Node, NodeFlags, SourceFile};
use tsrs_core::P;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Scope {
    block: P<Node>,
    /// A named function expression opens two scopes: one that holds its name alone, and inside it the one of its
    /// parameters and body.
    name_only: bool,
}

impl Scope {
    pub(super) fn of(block: P<Node>) -> Scope {
        Scope { block, name_only: false }
    }

    /// The scope that holds the name of the function expression `function`.
    pub(super) fn of_function_name(function: P<Node>) -> Scope {
        Scope { block: function, name_only: true }
    }

    /// True for the scope of a function type, of a signature or of a mapped type: an `infer` inside one declares
    /// its type parameter in the conditional type around them.
    fn passes_inferred_types(self) -> bool {
        match self.block.kind() {
            Kind::MethodSignature
            | Kind::CallSignature
            | Kind::ConstructSignature
            | Kind::FunctionType
            | Kind::ConstructorType
            | Kind::MappedType => true,
            Kind::GetAccessor | Kind::SetAccessor => is_type_member(self.block),
            _ => false,
        }
    }
}

/// A scope around a node.
#[derive(Clone, Copy)]
pub(super) struct Enclosing {
    pub(super) scope: Scope,
    /// Where the body of the function starts, when the node is in the parameters, the type parameters or the
    /// return type of a function: what the body alone declares is not visible from there.
    pub(super) hidden_from: Option<u32>,
}

/// The scopes around a node, innermost first.
pub(super) struct Enclosings {
    below: P<Node>,
    /// The child of `below` on the way down to the node.
    further_below: Option<P<Node>>,
    name_scope: Option<Scope>,
}

pub(super) fn enclosing(node: P<Node>) -> Enclosings {
    Enclosings { below: node, further_below: None, name_scope: None }
}

impl Iterator for Enclosings {
    type Item = Enclosing;

    fn next(&mut self) -> Option<Enclosing> {
        if let Some(scope) = self.name_scope.take() {
            return Some(Enclosing { scope, hidden_from: None });
        }
        loop {
            let child = self.below;
            let node = child.parent()?;
            let grandchild = self.further_below.replace(child);
            self.below = node;
            if let Some(found) = scope_of(node, child, grandchild) {
                if node.kind() == Kind::FunctionExpression && node.name().is_some() {
                    self.name_scope = Some(Scope::of_function_name(node));
                }
                return Some(found);
            }
        }
    }
}

/// The scope that `node` opens around its child `child`, when it opens one there.
fn scope_of(node: P<Node>, child: P<Node>, grandchild: Option<P<Node>>) -> Option<Enclosing> {
    let opened = match node.kind() {
        Kind::SourceFile
        | Kind::ModuleBlock
        | Kind::CaseBlock
        | Kind::CatchClause
        | Kind::ClassStaticBlockDeclaration
        | Kind::ForStatement
        | Kind::ForInStatement
        | Kind::ForOfStatement
        | Kind::MappedType => true,
        // The body of a function or of a static block is in the scope of its owner.
        Kind::Block => !node.parent().is_some_and(|owner| {
            is_function(owner.kind()) || owner.kind() == Kind::ClassStaticBlockDeclaration
        }),
        Kind::FunctionDeclaration
        | Kind::FunctionExpression
        | Kind::ArrowFunction
        | Kind::MethodDeclaration
        | Kind::Constructor
        | Kind::GetAccessor
        | Kind::SetAccessor => return function_scope(node, child, grandchild),
        // A computed name is read outside the scope, as the decorators of a class are.
        Kind::MethodSignature
        | Kind::CallSignature
        | Kind::ConstructSignature
        | Kind::FunctionType
        | Kind::ConstructorType
        | Kind::ClassDeclaration
        | Kind::ClassExpression
        | Kind::InterfaceDeclaration
        | Kind::TypeAliasDeclaration
        | Kind::EnumDeclaration => node.name() != Some(child) && child.kind() != Kind::Decorator,
        // The false branch does not see what the condition infers.
        Kind::ConditionalType => node.as_conditional_type_node().false_type != child,
        _ => false,
    };
    opened.then_some(Enclosing { scope: Scope::of(node), hidden_from: None })
}

fn function_scope(
    function: P<Node>,
    child: P<Node>,
    grandchild: Option<P<Node>>,
) -> Option<Enclosing> {
    if function.name() == Some(child) || child.kind() == Kind::Decorator {
        return None;
    }
    let body = function.body();
    // The scope manager reads the decorators of the parameters of a method before it opens the scope of the method.
    if body.is_some()
        && child.kind() == Kind::Parameter
        && grandchild.is_some_and(|below| below.kind() == Kind::Decorator)
        && function.parent().is_some_and(|owner| {
            matches!(owner.kind(), Kind::ClassDeclaration | Kind::ClassExpression)
        })
    {
        return None;
    }
    let hidden_from = body.filter(|body| *body != child).map(start);
    Some(Enclosing { scope: Scope::of(function), hidden_from })
}

/// The scope in which a declaration at `node` declares its name.
pub(super) fn declaring_scope(node: P<Node>) -> Option<Scope> {
    enclosing(node).next().map(|found| found.scope)
}

/// The scope in which a `var` at `node` declares its name: the function, the namespace body, the static block or
/// the file around it.
pub(super) fn variable_scope(node: P<Node>) -> Option<Scope> {
    let mut current = node.parent();
    while let Some(ancestor) = current {
        let kind = ancestor.kind();
        if is_function(kind)
            || matches!(
                kind,
                Kind::SourceFile | Kind::ModuleBlock | Kind::ClassStaticBlockDeclaration
            )
        {
            return Some(Scope::of(ancestor));
        }
        current = ancestor.parent();
    }
    None
}

/// The scope in which `infer X` declares `X`: the one around it, or the conditional type that scopes of function
/// types and mapped types alone separate from it.
pub(super) fn inferred_type_scope(infer: P<Node>) -> Option<Scope> {
    let mut scopes = enclosing(infer).map(|found| found.scope);
    let nearest = scopes.next()?;
    if !nearest.passes_inferred_types() {
        return Some(nearest);
    }
    let outer = scopes.find(|scope| !scope.passes_inferred_types());
    Some(outer.filter(|scope| scope.block.kind() == Kind::ConditionalType).unwrap_or(nearest))
}

/// True for a function that has parameters of its own and can have a body.
pub(super) fn is_function(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::FunctionDeclaration
            | Kind::FunctionExpression
            | Kind::ArrowFunction
            | Kind::MethodDeclaration
            | Kind::Constructor
            | Kind::GetAccessor
            | Kind::SetAccessor
    )
}

/// True for an accessor that an interface or a type literal declares.
fn is_type_member(node: P<Node>) -> bool {
    node.parent()
        .is_some_and(|owner| matches!(owner.kind(), Kind::InterfaceDeclaration | Kind::TypeLiteral))
}

/// Where `node` starts, with the trivia before it: enough to order nodes that do not overlap.
pub(super) fn start(node: P<Node>) -> u32 {
    node.pos().expect_source("lint nodes are parsed source nodes")
}

/// True when Oxlint reads `file` as a script, whose declarations are in the global scope.
///
/// The extension decides for `.mjs` and `.mts` (modules), and for `.cjs` and `.cts` (`CommonJS`, whose
/// declarations are in a function scope). Any other file is a module when it has an `import` declaration, an
/// `export`, `import.meta`, or an `await` outside functions. Oxlint 1.93 keeps a file a script when its only
/// module syntax is `import a = require('b')`.
pub(super) fn is_script(file: P<SourceFile>) -> bool {
    let name = file.file_name();
    if [".mjs", ".mts", ".cjs", ".cts"].iter().any(|extension| name.ends_with(extension)) {
        return false;
    }
    let has_module_statement = file.statements().nodes().iter().any(|statement| {
        !statement.flags().intersects(NodeFlags::Reparsed)
            && (matches!(
                statement.kind(),
                Kind::ImportDeclaration
                    | Kind::ExportDeclaration
                    | Kind::ExportAssignment
                    | Kind::NamespaceExportDeclaration
            ) || statement.modifier_flags().intersects(ModifierFlags::Export))
    });
    !has_module_statement && !has_module_expression(file.as_node(), true)
}

/// True when `node` contains `import.meta`, or an `await` that no function contains (`top_level`).
fn has_module_expression(node: P<Node>, top_level: bool) -> bool {
    node.for_each_child(&mut |child| match child.kind() {
        Kind::MetaProperty => child.as_meta_property().keyword_token == Kind::ImportKeyword,
        Kind::AwaitExpression if top_level => true,
        Kind::VariableDeclarationList | Kind::ForOfStatement
            if top_level && is_awaited_statement(child) =>
        {
            true
        }
        kind => has_module_expression(child, top_level && !is_function(kind)),
    })
}

/// True for `await using a = b` and `for await (a of b)`.
fn is_awaited_statement(node: P<Node>) -> bool {
    if node.kind() == Kind::ForOfStatement {
        node.as_for_in_or_of_statement().await_modifier.is_some()
    } else {
        node.flags() & NodeFlags::BlockScoped == NodeFlags::AwaitUsing
    }
}
