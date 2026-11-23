//! What the JS rule asks of the `ESTree`, answered on the TypeScript AST.
//!
//! The `ESTree` has no node for parentheses, and it has patterns (`ObjectPattern`, `AssignmentPattern`) where the
//! TypeScript AST keeps the expressions of a destructuring assignment.

use tsrs_ast::{Kind, Node, NodeFlags};
use tsrs_core::P;

/// The parent of `node` in the `ESTree`, with the child that leads to `node`.
///
/// Parentheses are not nodes there, and neither is the cast that the parser makes of a `JSDoc` `@type` or
/// `@satisfies` in a JS file.
pub(super) fn parent_of(node: P<Node>) -> Option<(P<Node>, P<Node>)> {
    let mut child = node;
    loop {
        let parent = child.parent()?;
        if parent.kind() != Kind::ParenthesizedExpression && !is_jsdoc_cast(parent) {
            return Some((child, parent));
        }
        child = parent;
    }
}

fn is_jsdoc_cast(node: P<Node>) -> bool {
    matches!(node.kind(), Kind::AsExpression | Kind::SatisfiesExpression)
        && node.type_node().is_some_and(|ty| ty.flags().intersects(NodeFlags::Reparsed))
}

/// Where the identifier `name` ends in the `ESTree`: the name of a variable or of a parameter has its `?`, its `!`
/// and its type annotation in its range (`a?: string`). A rest parameter gives them to its `RestElement`, and
/// what the parser reads from a `JSDoc` comment is not there.
pub(super) fn end_of_identifier(name: P<Node>) -> u32 {
    let written =
        |part: Option<P<Node>>| part.filter(|part| !part.flags().intersects(NodeFlags::Reparsed));
    let declaration = name.parent().filter(|declaration| declaration.name() == Some(name));
    let last = match declaration.map(|declaration| (declaration.kind(), declaration)) {
        Some((Kind::VariableDeclaration, declaration)) => {
            let variable = declaration.as_variable_declaration();
            written(variable.type_()).or_else(|| written(variable.exclamation_token()))
        }
        Some((Kind::Parameter, declaration)) => {
            let parameter = declaration.as_parameter_declaration();
            if parameter.dot_dot_dot_token().is_some() {
                None
            } else {
                written(parameter.type_()).or_else(|| written(parameter.question_token()))
            }
        }
        _ => None,
    };
    last.unwrap_or(name).end().expect_source("lint nodes are parsed source nodes")
}

/// True for the array or object literal that a destructuring assignment writes to: `[a, b] = c`, the `{ d }` of
/// `({ e: { d } } = f)`, the left of `for ([g] of h)`.
pub(super) fn is_assignment_pattern(literal: P<Node>) -> bool {
    let Some((child, parent)) = parent_of(literal) else {
        return false;
    };
    match parent.kind() {
        Kind::BinaryExpression => is_plain_assignment_to(parent, child),
        Kind::ForInStatement | Kind::ForOfStatement => {
            parent.as_for_in_or_of_statement().initializer == child
        }
        Kind::ArrayLiteralExpression => is_assignment_pattern(parent),
        Kind::PropertyAssignment => {
            parent.as_property_assignment().initializer.get() == child && is_in_pattern(parent)
        }
        Kind::SpreadElement | Kind::SpreadAssignment => is_in_pattern(parent),
        _ => false,
    }
}

/// True for the element or the property of a destructuring assignment.
fn is_in_pattern(member: P<Node>) -> bool {
    member.parent().is_some_and(|literal| {
        matches!(literal.kind(), Kind::ArrayLiteralExpression | Kind::ObjectLiteralExpression)
            && is_assignment_pattern(literal)
    })
}

fn is_plain_assignment_to(binary: P<Node>, left: P<Node>) -> bool {
    let binary = binary.as_binary_expression();
    binary.left == left && binary.operator_token.kind() == Kind::EqualsToken
}

/// True for `a = b` where it gives a default value in a destructuring assignment: `[a = b] = c`,
/// `({ d: a = b } = c)`. The `ESTree` has an `AssignmentPattern` there.
fn is_default_in_pattern(binary: P<Node>) -> bool {
    binary.as_binary_expression().operator_token.kind() == Kind::EqualsToken
        && parent_of(binary).is_some_and(|(child, parent)| match parent.kind() {
            Kind::ArrayLiteralExpression => is_assignment_pattern(parent),
            Kind::PropertyAssignment => {
                parent.as_property_assignment().initializer.get() == child && is_in_pattern(parent)
            }
            _ => false,
        })
}

/// The JS rule's `isAssignmentTarget`, for the member access `member`: true for the target of an assignment, of
/// a default value or of a destructuring.
pub(super) fn is_assignment_target(member: P<Node>) -> bool {
    let Some((child, parent)) = parent_of(member) else {
        return false;
    };
    match parent.kind() {
        Kind::BinaryExpression => {
            let binary = parent.as_binary_expression();
            binary.left == child && tsrs_ast::is_assignment_operator(binary.operator_token.kind())
        }
        Kind::ArrayLiteralExpression => is_assignment_pattern(parent),
        Kind::PropertyAssignment => {
            parent.as_property_assignment().initializer.get() == child && is_in_pattern(parent)
        }
        Kind::SpreadElement | Kind::SpreadAssignment => is_in_pattern(parent),
        _ => false,
    }
}

/// True where the `ESTree` parent of the reference `node` is a call or a `new` (as callee or as argument), or where
/// `node` is a default value: `ESLint` keeps those unreported "for backward compatibility".
pub(super) fn is_call_or_default(node: P<Node>) -> bool {
    let Some((child, parent)) = parent_of(node) else {
        return false;
    };
    match parent.kind() {
        // `import(a)` is an `ImportExpression`.
        Kind::CallExpression => {
            parent.as_call_expression().expression.kind() != Kind::ImportKeyword
        }
        Kind::NewExpression => true,
        Kind::Parameter | Kind::BindingElement => parent.initializer() == Some(child),
        Kind::ShorthandPropertyAssignment => {
            parent.as_shorthand_property_assignment().object_assignment_initializer.get()
                == Some(child)
        }
        Kind::BinaryExpression => {
            parent.as_binary_expression().right.get() == child && is_default_in_pattern(parent)
        }
        _ => false,
    }
}

/// The JS rule's `equalsToOriginalName`, for a reference: true for the value of a property that has its name,
/// in an object or in a destructuring assignment (`{ a_b }`, `{ a_b: a_b }`, `({ a_b = 1 } = c)`).
pub(super) fn reference_keeps_property_name(node: P<Node>) -> bool {
    let Some((mut child, mut parent)) = parent_of(node) else {
        return false;
    };
    if parent.kind() == Kind::ShorthandPropertyAssignment {
        return parent.name() == Some(node);
    }
    // `a_b = 1` is the value of the property where it is a default.
    if parent.kind() == Kind::BinaryExpression {
        if parent.as_binary_expression().left != child || !is_default_in_pattern(parent) {
            return false;
        }
        let Some(above) = parent_of(parent) else {
            return false;
        };
        (child, parent) = above;
    }
    parent.kind() == Kind::PropertyAssignment
        && parent.as_property_assignment().initializer.get() == child
        && parent
            .name()
            .is_some_and(|key| key.kind() == Kind::Identifier && key.text() == node.text())
}

/// The JS rule's `equalsToOriginalName`, for a declared name: true for `{ a_b }` and `{ a_b: a_b }` in a binding
/// pattern, and for `import { a_b }`.
pub(super) fn binding_keeps_original_name(name: P<Node>) -> bool {
    let Some(parent) = name.parent() else {
        return false;
    };
    match parent.kind() {
        Kind::BindingElement => {
            let element = parent.as_binding_element();
            element.dot_dot_dot_token().is_none()
                && parent
                    .parent()
                    .is_some_and(|pattern| pattern.kind() == Kind::ObjectBindingPattern)
                && element
                    .property_name()
                    .is_none_or(|key| key.kind() == Kind::Identifier && key.text() == name.text())
        }
        // The imported name is an identifier or a string.
        Kind::ImportSpecifier => parent.property_name().is_none_or(|imported| {
            matches!(imported.kind(), Kind::Identifier | Kind::StringLiteral)
                && imported.text() == name.text()
        }),
        _ => false,
    }
}

/// The JS rule's `isImportAttributeKey`, for the key of `property` or for its value when it is a shorthand: true
/// in the options of a dynamic import, `import('x', { with: { some_key: 'y' } })`.
pub(super) fn is_import_option(property: P<Node>) -> bool {
    if property.name().is_none_or(|key| key.kind() == Kind::ComputedPropertyName) {
        return false;
    }
    let Some(object) = property.parent().filter(|o| o.kind() == Kind::ObjectLiteralExpression)
    else {
        return false;
    };
    let Some((child, owner)) = parent_of(object) else {
        return false;
    };
    match owner.kind() {
        Kind::CallExpression => {
            owner.as_call_expression().expression.kind() == Kind::ImportKeyword
                && owner.arguments().get(1) == Some(&child)
        }
        Kind::PropertyAssignment => {
            owner.as_property_assignment().initializer.get() == child && is_import_option(owner)
        }
        _ => false,
    }
}
