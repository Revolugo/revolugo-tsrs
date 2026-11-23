//! `js/camelcase`: reports the identifiers that have an underscore inside their name and are not all upper case:
//! variables, functions, classes, parameters, imports, exported names, labels, and the properties and members that
//! code declares or assigns.
//!
//! Port of `src/rules/core/camelcase.ts` of `revolugo-lint-plugins`, itself a port of `ESLint`'s `camelcase`.
//!
//! The JS rule reads the variables and the references of typescript-eslint's scope manager. Here each underscored
//! identifier is classified by its parent, as that scope manager does: the name of a declaration is recorded in
//! its scope (`scope`), and a reference waits for the end of the file, when every declaration it can resolve to is
//! known. A reference is reported unless it resolves to what the JS rule never lists: a type, an enum, a
//! namespace, an `import =`, the parameter of a signature.
//!
//! `ignoreGlobals` is accepted and changes nothing: it spares the references to the globals that the Oxlint
//! configuration declares, which this rule does not receive. None of Oxlint's built-in globals is underscored.

mod estree;
mod scope;

use std::sync::{Arc, Mutex, PoisonError};

use rustc_hash::FxHashMap;
use tsrs_ast::{Kind, ModifierFlags, Node, NodeFlags, SourceFile};
use tsrs_core::P;
use tsrs_core::json::Value;

use self::scope::Scope;
use crate::protocol::RuleMessage;
use crate::rule::{Listener, Rule, RuleContext, trim_node_range};

const RULE: &str = "js/camelcase";

pub struct Camelcase {
    options: Options,
    /// Whether an underscored name is reported, for each one that the `allow` option was asked about.
    allowed: FxHashMap<&'static str, bool>,
    /// The variables that have an underscored name, in the order of their first declaration, and where each one
    /// is in the list.
    variables: Vec<Variable>,
    variable_of: FxHashMap<(Scope, &'static str), usize>,
    references: Vec<Reference>,
    /// The identifiers to report. One can be found twice: a shorthand property is a key and a reference.
    found: Vec<P<Node>>,
}

struct Options {
    allow: Arc<Allow>,
    ignore_destructuring: bool,
    ignore_imports: bool,
    /// `properties: "always"`, the default.
    properties: bool,
}

/// The `allow` option: a name is allowed when it is an entry, or when an entry matches it as a regular expression
/// of JavaScript with the `u` flag.
struct Allow {
    entries: Vec<(String, regress::Regex)>,
}

/// The `allow` lists compiled so far. A rule is made for each file, and a configuration has few distinct lists.
static ALLOW_LISTS: Mutex<Vec<Arc<Allow>>> = Mutex::new(Vec::new());

impl Allow {
    fn of(entries: &[&str]) -> Result<Arc<Allow>, String> {
        let mut lists = ALLOW_LISTS.lock().unwrap_or_else(PoisonError::into_inner);
        let known = lists.iter().find(|list| {
            list.entries.len() == entries.len()
                && list.entries.iter().zip(entries).all(|((text, _), entry)| text == entry)
        });
        if let Some(list) = known {
            return Ok(Arc::clone(list));
        }
        let compiled = entries
            .iter()
            .map(|entry| {
                let flags = regress::Flags { unicode: true, ..Default::default() };
                regress::Regex::with_flags(entry, flags)
                    .map(|pattern| ((*entry).to_owned(), pattern))
                    .map_err(|error| format!("{RULE}: invalid pattern {entry:?} in allow: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let list = Arc::new(Allow { entries: compiled });
        lists.push(Arc::clone(&list));
        Ok(list)
    }

    fn allows(&self, name: &str) -> bool {
        self.entries.iter().any(|(text, pattern)| text == name || pattern.find(name).is_some())
    }
}

impl Options {
    fn read(options: &[Value]) -> Result<Options, String> {
        let object = match options {
            [] => None,
            [Value::Object(object)] => Some(object),
            [_] => return Err(format!("{RULE}: the options are an object")),
            _ => return Err(format!("{RULE}: takes one options object")),
        };
        let mut allow = Vec::new();
        let mut ignore_destructuring = false;
        let mut ignore_imports = false;
        let mut properties = true;
        for (key, value) in object.into_iter().flat_map(|object| object.iter()) {
            match (key, value) {
                ("allow", Value::Array(entries)) => {
                    for entry in entries {
                        let Value::String(entry) = entry else {
                            return Err(format!("{RULE}: allow is an array of strings"));
                        };
                        if allow.contains(&entry.as_str()) {
                            return Err(format!("{RULE}: allow has {entry:?} twice"));
                        }
                        allow.push(entry.as_str());
                    }
                }
                ("allow", _) => return Err(format!("{RULE}: allow is an array of strings")),
                ("ignoreDestructuring", Value::Bool(value)) => ignore_destructuring = *value,
                ("ignoreImports", Value::Bool(value)) => ignore_imports = *value,
                ("ignoreGlobals", Value::Bool(_)) => {}
                ("ignoreDestructuring" | "ignoreImports" | "ignoreGlobals", _) => {
                    return Err(format!("{RULE}: {key} is a boolean"));
                }
                ("properties", Value::String(value)) if value == "always" => properties = true,
                ("properties", Value::String(value)) if value == "never" => properties = false,
                ("properties", _) => {
                    return Err(format!("{RULE}: properties is \"always\" or \"never\""));
                }
                _ => return Err(format!("{RULE}: unknown option {key:?}")),
            }
        }
        Ok(Options { allow: Allow::of(&allow)?, ignore_destructuring, ignore_imports, properties })
    }
}

/// The JS rule's `isUnderscored`: `A_CONSTANT` and the underscores around a name are not reported.
fn is_underscored(name: &str) -> bool {
    let body = name.trim_matches('_');
    body.contains('_')
        && if body.is_ascii() {
            body.bytes().any(|byte| byte.is_ascii_lowercase())
        } else {
            body.to_uppercase() != body
        }
}

/// What the scope manager calls a variable: the declarations of one name in one scope.
struct Variable {
    /// The first declared identifier, which is the one that the JS rule reports, with its place in the order of
    /// declaration. A string that names an enum member is a declaration without identifier.
    first: Option<(u32, Declared)>,
    /// What the declarations are together.
    kinds: u8,
}

#[derive(Clone, Copy)]
struct Declared {
    name: P<Node>,
    /// `{ a_b }` in a binding pattern or `import { a_b }`: what `ignoreDestructuring` and `ignoreImports` spare.
    keeps_original_name: bool,
}

/// A declaration that a value reference resolves to.
const VALUE: u8 = 1;
/// A declaration that a type reference resolves to.
const TYPE: u8 = 1 << 1;
/// A declaration that the JS rule lists: a variable, a function or a class with a body, a parameter of one, the
/// parameter of a `catch`. It reports the variable and its references.
const LISTED: u8 = 1 << 2;
/// The binding of an `import` declaration, reported as a listed declaration is but spared by `ignoreImports`.
const IMPORTED: u8 = 1 << 3;

struct Reference {
    node: P<Node>,
    /// `VALUE`, `TYPE` or both: the declarations that it resolves to.
    kinds: u8,
}

impl Camelcase {
    pub fn new(options: &[Value]) -> Result<Self, String> {
        Ok(Self {
            options: Options::read(options)?,
            allowed: FxHashMap::default(),
            variables: Vec::new(),
            variable_of: FxHashMap::default(),
            references: Vec::new(),
            found: Vec::new(),
        })
    }

    /// The JS rule's `isGoodName`, negated.
    fn is_bad_name(&mut self, name: &'static str) -> bool {
        if !is_underscored(name) {
            return false;
        }
        let allow = &self.options.allow;
        allow.entries.is_empty() || !*self.allowed.entry(name).or_insert_with(|| allow.allows(name))
    }

    fn identifier(&mut self, node: P<Node>) {
        let name = node.text();
        if !self.is_bad_name(name)
            || node.flags().intersects(NodeFlags::Reparsed | NodeFlags::JSDoc)
        {
            return;
        }
        let Some(parent) = node.parent() else {
            return;
        };
        let is_name = parent.name() == Some(node);
        match parent.kind() {
            Kind::VariableDeclaration | Kind::BindingElement | Kind::Parameter if is_name => {
                self.declare_binding(node, parent);
            }
            // The key of a binding pattern is not a variable.
            Kind::BindingElement if parent.property_name() == Some(node) => {}
            Kind::FunctionDeclaration if is_name => {
                let listed = if parent.body().is_some() { LISTED } else { 0 };
                self.declare(scope::declaring_scope(node), node, VALUE | listed);
            }
            Kind::FunctionExpression if is_name => {
                self.declare(Some(Scope::of_function_name(parent)), node, VALUE | LISTED);
            }
            // A class declares its name around it, and again inside it.
            Kind::ClassDeclaration | Kind::ClassExpression if is_name => {
                if parent.kind() == Kind::ClassDeclaration {
                    self.declare(scope::declaring_scope(node), node, VALUE | TYPE | LISTED);
                }
                self.declare(Some(Scope::of(parent)), node, VALUE | TYPE | LISTED);
            }
            Kind::ImportClause | Kind::NamespaceImport | Kind::ImportSpecifier if is_name => {
                self.declare(scope::declaring_scope(node), node, VALUE | TYPE | IMPORTED);
            }
            // The imported name, and the name that `import('a').b_c` qualifies, are not variables of the file.
            Kind::ImportSpecifier | Kind::ImportType => {}
            Kind::ImportEqualsDeclaration | Kind::EnumDeclaration | Kind::EnumMember if is_name => {
                self.declare(scope::declaring_scope(node), node, VALUE | TYPE);
            }
            Kind::ModuleDeclaration if is_name => {
                if is_single_namespace(parent) {
                    self.declare(scope::declaring_scope(node), node, VALUE | TYPE);
                }
            }
            Kind::InterfaceDeclaration | Kind::TypeAliasDeclaration if is_name => {
                self.declare(scope::declaring_scope(node), node, TYPE);
            }
            Kind::TypeParameter if is_name => self.declare_type_parameter(node, parent),
            Kind::MethodDeclaration | Kind::GetAccessor | Kind::SetAccessor if is_name => {
                if is_method_definition(parent) {
                    self.property_key(node, parent);
                }
            }
            Kind::PropertyDeclaration if is_name => {
                if is_property_definition(parent) {
                    self.property_key(node, parent);
                }
            }
            Kind::PropertyAssignment if is_name => self.property_key(node, parent),
            Kind::ShorthandPropertyAssignment if is_name => {
                self.property_key(node, parent);
                self.reference(node, VALUE);
            }
            Kind::PropertyAccessExpression if is_name => {
                if self.options.properties && estree::is_assignment_target(parent) {
                    self.found.push(node);
                }
            }
            // The names that are neither variables nor references.
            Kind::PropertySignature
            | Kind::MethodSignature
            | Kind::JsxAttribute
            | Kind::NamedTupleMember
            | Kind::ImportAttribute
                if is_name => {}
            Kind::ExportSpecifier => self.export_specifier(node, parent),
            Kind::NamespaceExport
            | Kind::LabeledStatement
            | Kind::BreakStatement
            | Kind::ContinueStatement => self.found.push(node),
            // `export default a_b` and `export = a_b` export what `a_b` is, a value or a type.
            Kind::ExportAssignment => self.reference(node, VALUE | TYPE),
            Kind::ParenthesizedExpression if is_exported_expression(node) => {
                self.reference(node, VALUE | TYPE);
            }
            Kind::TypeReference => self.reference(node, TYPE),
            Kind::QualifiedName => self.qualified_name(node, parent),
            Kind::ExpressionWithTypeArguments => {
                self.reference(node, if is_type_heritage(parent) { TYPE } else { VALUE });
            }
            Kind::PropertyAccessExpression => {
                self.reference(node, if is_in_type_heritage(parent) { TYPE } else { VALUE });
            }
            Kind::JsxOpeningElement | Kind::JsxSelfClosingElement | Kind::JsxClosingElement => {
                if is_component_name(name) {
                    self.reference(node, VALUE);
                }
            }
            Kind::JsxNamespacedName => {
                if parent.parent().is_some_and(|owner| owner.kind() != Kind::JsxAttribute) {
                    self.reference(node, VALUE);
                }
            }
            _ => self.reference(node, VALUE),
        }
    }

    /// The name of a variable, of a parameter or of an element of their binding patterns.
    fn declare_binding(&mut self, name: P<Node>, parent: P<Node>) {
        let mut declaration = parent;
        while declaration.kind() == Kind::BindingElement {
            let Some(outer) = declaration.parent().and_then(|pattern| pattern.parent()) else {
                return;
            };
            declaration = outer;
        }
        let Some(owner) = declaration.parent() else {
            return;
        };
        let (scope, kinds) = match (declaration.kind(), owner.kind()) {
            (Kind::VariableDeclaration, Kind::CatchClause) => {
                (Some(Scope::of(owner)), VALUE | LISTED)
            }
            (Kind::VariableDeclaration, _) => {
                let scope = if owner.flags().intersects(NodeFlags::BlockScoped) {
                    scope::declaring_scope(declaration)
                } else {
                    scope::variable_scope(declaration)
                };
                (scope, VALUE | LISTED)
            }
            // The parameter of an index signature is not a variable.
            (Kind::Parameter, Kind::IndexSignature) => return,
            (Kind::Parameter, _) => {
                let listed = scope::is_function(owner.kind()) && owner.body().is_some();
                (Some(Scope::of(owner)), if listed { VALUE | LISTED } else { VALUE })
            }
            _ => return,
        };
        self.declare(scope, name, kinds);
    }

    fn declare_type_parameter(&mut self, name: P<Node>, parameter: P<Node>) {
        let Some(owner) = parameter.parent() else {
            return;
        };
        if owner.kind() == Kind::InferType {
            self.declare(scope::inferred_type_scope(owner), name, TYPE);
            return;
        }
        // The scope manager declares the type parameters of a function after its parameters.
        let order = if scope::is_function(owner.kind()) {
            owner.parameter_list().map(|parameters| parameters.end().expect_source("parsed list"))
        } else {
            None
        };
        let scope = scope::declaring_scope(name);
        self.declare_at(scope, order.unwrap_or_else(|| scope::start(name)), name, TYPE);
    }

    fn declare(&mut self, scope: Option<Scope>, name: P<Node>, kinds: u8) {
        self.declare_at(scope, scope::start(name), name, kinds);
    }

    fn variable(&mut self, key: (Scope, &'static str)) -> &mut Variable {
        let index = *self.variable_of.entry(key).or_insert_with(|| {
            self.variables.push(Variable { first: None, kinds: 0 });
            self.variables.len() - 1
        });
        &mut self.variables[index]
    }

    /// Adds a declaration of `name` to its variable. `order` places it among the declarations of the variable.
    fn declare_at(&mut self, scope: Option<Scope>, order: u32, name: P<Node>, kinds: u8) {
        let Some(scope) = scope else {
            return;
        };
        let variable = self.variable((scope, name.text()));
        variable.kinds |= kinds;
        if variable.first.is_none_or(|(first, _)| order < first) {
            let keeps_original_name = estree::binding_keeps_original_name(name);
            variable.first = Some((order, Declared { name, keeps_original_name }));
        }
    }

    /// The member of an enum that a string names: `enum A { 'b_c' }`. The scope manager declares it as it does
    /// an identifier, so a reference in the enum resolves to it.
    fn enum_member(&mut self, member: P<Node>) {
        let Some(name) = member.name().filter(|name| name.kind() == Kind::StringLiteral) else {
            return;
        };
        let text = name.text();
        if self.is_bad_name(text)
            && let Some(scope) = member.parent().map(Scope::of)
        {
            self.variable((scope, text)).kinds |= VALUE | TYPE;
        }
    }

    /// The private name of a member of a class: `#a_b() {}`, `#a_b = 1`.
    fn private_identifier(&mut self, node: P<Node>) {
        let name = node.text();
        if !self.is_bad_name(name.strip_prefix('#').unwrap_or(name)) {
            return;
        }
        let Some(member) = node.parent().filter(|member| member.name() == Some(node)) else {
            return;
        };
        let is_key = match member.kind() {
            Kind::MethodDeclaration | Kind::GetAccessor | Kind::SetAccessor => {
                is_method_definition(member)
            }
            Kind::PropertyDeclaration => is_property_definition(member),
            _ => false,
        };
        if is_key && self.options.properties {
            self.found.push(node);
        }
    }

    /// The JS rule's `PROPERTY_KEYS`: the key of a property of an object, not of a destructuring assignment, and
    /// the key of a member of a class.
    fn property_key(&mut self, key: P<Node>, property: P<Node>) {
        if !self.options.properties {
            return;
        }
        let in_object =
            property.parent().filter(|object| object.kind() == Kind::ObjectLiteralExpression);
        if in_object.is_some_and(estree::is_assignment_pattern)
            || estree::is_import_option(property)
        {
            return;
        }
        self.found.push(key);
    }

    fn export_specifier(&mut self, node: P<Node>, specifier: P<Node>) {
        let export = specifier.as_export_specifier();
        // The JS rule's `EXPORTED_NAMES`. In `export { a_b }` the reference to `a_b` is at the same place.
        if export.name == node {
            self.found.push(node);
            return;
        }
        // `export { a_b as c } from 'd'` reads no variable.
        let Some(declaration) = specifier.parent().and_then(|exports| exports.parent()) else {
            return;
        };
        if declaration.kind() != Kind::ExportDeclaration {
            return;
        }
        let declaration = declaration.as_export_declaration();
        if declaration.module_specifier.is_none() {
            let type_only = export.is_type_only || declaration.is_type_only;
            self.reference(node, if type_only { TYPE } else { VALUE | TYPE });
        }
    }

    /// The left of `a_b.c` in a type, in `typeof a_b.c` and in `import d = a_b.c`.
    fn qualified_name(&mut self, node: P<Node>, mut qualified: P<Node>) {
        if qualified.as_qualified_name().left != node {
            return;
        }
        let mut owner = qualified.parent();
        while let Some(outer) = owner.filter(|outer| outer.kind() == Kind::QualifiedName) {
            if outer.as_qualified_name().left != qualified {
                return;
            }
            qualified = outer;
            owner = outer.parent();
        }
        match owner.map(|owner| owner.kind()) {
            Some(Kind::TypeReference) => self.reference(node, TYPE),
            Some(Kind::TypeQuery | Kind::ImportEqualsDeclaration) => self.reference(node, VALUE),
            _ => {}
        }
    }

    /// The JS rule's `reportReference`, before the reference is resolved.
    fn reference(&mut self, node: P<Node>, kinds: u8) {
        if estree::is_call_or_default(node)
            || (self.options.ignore_destructuring && estree::reference_keeps_property_name(node))
        {
            return;
        }
        if let Some(property) = node.parent()
            && property.kind() == Kind::ShorthandPropertyAssignment
            && estree::is_import_option(property)
        {
            return;
        }
        self.references.push(Reference { node, kinds });
    }

    /// The variable that `reference` resolves to, as the scope manager resolves it.
    fn resolve(&self, reference: &Reference) -> Option<&Variable> {
        let name = reference.node.text();
        scope::enclosing(reference.node).find_map(|found| {
            let variable = &self.variables[*self.variable_of.get(&(found.scope, name))?];
            // What the body of a function declares is not visible from its parameters.
            let hidden = found
                .hidden_from
                .is_some_and(|body| variable.first.is_some_and(|(order, _)| order >= body));
            (variable.kinds & reference.kinds != 0 && !hidden).then_some(variable)
        })
    }

    /// The variable that Oxlint gives a reference that the scope manager left unresolved.
    ///
    /// Oxlint adds the globals of its configuration to the global scope, then resolves each unresolved reference
    /// to the variable of that scope that has its name, whatever the variable is. In a script, the declarations of
    /// the file are in that scope too, so `interface a_b {}` resolves the value `a_b`.
    fn resolve_as_global(
        &self,
        reference: &Reference,
        file: P<SourceFile>,
        is_script: &mut Option<bool>,
    ) -> Option<&Variable> {
        let index = self.variable_of.get(&(Scope::of(file.as_node()), reference.node.text()))?;
        let variable = &self.variables[*index];
        is_script.get_or_insert_with(|| scope::is_script(file)).then_some(variable)
    }

    /// Reports what the file has, once all of it was read: a name is reported for the variable it declares first,
    /// and a reference for what it resolves to.
    fn finish(&mut self, context: &mut RuleContext<'_>) {
        let mut found = std::mem::take(&mut self.found);
        for variable in &self.variables {
            let Some((_, declared)) = variable.first else {
                continue;
            };
            let spared = |ignore: bool| ignore && declared.keeps_original_name;
            if (variable.kinds & LISTED != 0 && !spared(self.options.ignore_destructuring))
                || (variable.kinds & IMPORTED != 0 && !spared(self.options.ignore_imports))
            {
                found.push(declared.name);
            }
        }
        // A reference is reported unless it resolves to a variable that the JS rule does not list.
        let mut is_script = None;
        for reference in &self.references {
            let variable = self
                .resolve(reference)
                .or_else(|| self.resolve_as_global(reference, context.source_file, &mut is_script));
            if variable.is_none_or(|variable| variable.kinds & (LISTED | IMPORTED) != 0) {
                found.push(reference.node);
            }
        }
        found.sort_unstable_by_key(|node| scope::start(*node));
        found.dedup();
        for node in &found {
            let description = if node.kind() == Kind::PrivateIdentifier {
                format!("{} is not in camel case.", node.text())
            } else {
                format!("Identifier '{}' is not in camel case.", node.text())
            };
            let mut range = trim_node_range(context.source_file, *node);
            range.end = estree::end_of_identifier(*node);
            context.report_range(
                range,
                RuleMessage { id: "notCamelCase".into(), description, ..Default::default() },
            );
        }
    }
}

/// True for the name of a JSX element that the scope manager reads as a variable: one whose first UTF-16 unit is
/// its own upper case. `<div>` is an HTML element.
fn is_component_name(name: &str) -> bool {
    name.chars().next().is_some_and(|first| {
        first.len_utf16() > 1 || first.to_uppercase().eq(std::iter::once(first))
    })
}

/// True for `a` in `export default (a)`, which the `ESTree` has without the parentheses.
fn is_exported_expression(node: P<Node>) -> bool {
    estree::parent_of(node).is_some_and(|(_, parent)| parent.kind() == Kind::ExportAssignment)
}

/// True for `namespace A {}`. The `ESTree` gives `namespace A.B {}` one node, whose name is not an identifier, and
/// `declare global {}` declares no name.
fn is_single_namespace(module: P<Node>) -> bool {
    module.as_module_declaration().keyword != Kind::GlobalKeyword
        && module.body().is_none_or(|body| body.kind() != Kind::ModuleDeclaration)
        && module.parent().is_some_and(|parent| parent.kind() != Kind::ModuleDeclaration)
}

/// True for what the `ESTree` calls a `MethodDefinition` or the method of an object: not an abstract method, and
/// not the accessor of an interface or of a type literal.
fn is_method_definition(method: P<Node>) -> bool {
    !method.modifier_flags().intersects(ModifierFlags::Abstract)
        && method.parent().is_some_and(|owner| {
            matches!(
                owner.kind(),
                Kind::ClassDeclaration | Kind::ClassExpression | Kind::ObjectLiteralExpression
            )
        })
}

/// True for what the `ESTree` calls a `PropertyDefinition`: not an abstract property, and not an `accessor`.
fn is_property_definition(property: P<Node>) -> bool {
    !property.modifier_flags().intersects(ModifierFlags::Abstract | ModifierFlags::Accessor)
}

/// True for `a` in `implements a` and in the `extends a` of an interface, which name types. The `extends a` of a
/// class reads a value.
fn is_type_heritage(heritage: P<Node>) -> bool {
    heritage.parent().is_some_and(|clause| {
        clause.kind() == Kind::HeritageClause
            && (clause.as_heritage_clause().token == Kind::ImplementsKeyword
                || clause.parent().is_some_and(|owner| owner.kind() == Kind::InterfaceDeclaration))
    })
}

/// True for the `a.b` of `implements a.b.c`.
fn is_in_type_heritage(mut access: P<Node>) -> bool {
    while let Some(parent) = access.parent() {
        match parent.kind() {
            Kind::PropertyAccessExpression => access = parent,
            Kind::ExpressionWithTypeArguments => return is_type_heritage(parent),
            _ => return false,
        }
    }
    false
}

impl Rule for Camelcase {
    fn listens_to(&self, listener: Listener) -> bool {
        matches!(
            listener,
            Listener::Enter(
                Kind::Identifier | Kind::PrivateIdentifier | Kind::EnumMember | Kind::EndOfFile
            )
        )
    }

    fn visit(&mut self, _: Listener, node: P<Node>, context: &mut RuleContext<'_>) {
        match node.kind() {
            Kind::Identifier => self.identifier(node),
            Kind::PrivateIdentifier => self.private_identifier(node),
            Kind::EnumMember => self.enum_member(node),
            _ => self.finish(context),
        }
    }
}
