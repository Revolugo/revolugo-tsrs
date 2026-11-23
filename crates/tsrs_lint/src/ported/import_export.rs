//! `js/import-export`: a module exports a name once, and has one default export. TypeScript's overloads and
//! declaration merging (a namespace with a function, a class or an enum of the same name) are not duplicates, and
//! a type and a value may share a name.
//!
//! `export * from` counts for every name of the module it names, read from that module's source: a name that two of
//! them bring in, or one of them and the file itself, is reported, since TypeScript silently drops a name that two
//! `export *` give. An `export *` of a module without a named export is reported too.
//!
//! Port of `src/rules/import/export.ts` of `revolugo-lint-plugins`, itself a port of the `export` rule of
//! `eslint-plugin-import-x` (MIT license). The JS rule works on the `ESTree` that Oxlint gives it: the comments name
//! the `ESTree` node that a TypeScript node stands for where the two trees differ.
//!
//! The module that an `export *` names is resolved as the JS rule resolves it (`resolve`), not as the program does:
//! the JS resolver knows no `.ts` extension, no `paths` and no declaration file, and what resolves decides what is
//! reported. The exports of the resolved file are read from the tree that the program has already parsed when the
//! program has the file (`module_exports`).

mod component;
mod module_exports;
mod resolve;

use std::sync::Arc;

use tsrs_ast::{Kind, ModifierFlags, Node, NodeFlags};
use tsrs_core::P;
use tsrs_core::json::Value;

use crate::protocol::{Range, RuleMessage};
use crate::rule::{Listener, Rule, RuleContext, trim_node_range};

use module_exports::{ExportKind, ModuleExports, Modules};

/// The `ESTree` type of the parent of an exported node: it decides what merges with what.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Holder {
    ClassDeclaration,
    FunctionDeclaration,
    /// A function without a body: an overload, or a `declare function`.
    DeclareFunction,
    EnumDeclaration,
    InterfaceDeclaration,
    TypeAliasDeclaration,
    ModuleDeclaration,
    ImportEqualsDeclaration,
    VariableDeclarator,
    Property,
    ArrayPattern,
    AssignmentPattern,
    RestElement,
    ExportSpecifier,
    /// What holds the statement of an `export default` or an `export *`: the file, or a block.
    Statements(Kind),
}

/// What an `export default` exports, as far as the rule tells one from another.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DefaultExport {
    Function,
    Overload,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    /// The name of a declaration, of a variable or of an export specifier.
    Name,
    /// The statement of an `export default`.
    Default(DefaultExport),
    /// The statement of an `export * from`, by its index among those of the file.
    Star(usize),
}

/// A node that exports a name: the one that a duplicate is reported on.
#[derive(Clone, Copy)]
struct Site {
    node: P<Node>,
    holder: Holder,
    form: Form,
}

impl Site {
    fn is_star(&self) -> bool {
        matches!(self.form, Form::Star(_))
    }

    fn is_overload(&self) -> bool {
        self.form == Form::Default(DefaultExport::Overload)
            || (self.form == Form::Name && self.holder == Holder::DeclareFunction)
    }

    fn is_function(&self) -> bool {
        self.form == Form::Default(DefaultExport::Function)
            || (self.form == Form::Name && self.holder == Holder::FunctionDeclaration)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Value,
    Type,
    /// An `export *` that brings the name in: what the name is there is read when something else exports it too.
    Star,
}

/// One export of a name.
#[derive(Clone, Copy)]
struct Entry<'a> {
    /// The namespace block that holds the export, or none for the module itself.
    scope: Option<P<Node>>,
    name: &'a str,
    side: Side,
    site: Site,
}

impl<'a> Entry<'a> {
    /// What the exports that may be duplicates of one another share: a scope and a name.
    fn key(&self) -> (u32, &'a str) {
        let scope = self.scope.map_or(0, |scope| scope.pos().expect_source("parsed lint node"));
        (scope, self.name)
    }
}

/// An `export * from`.
struct Star {
    node: P<Node>,
    scope: Option<P<Node>>,
    source: P<Node>,
    type_only: bool,
}

/// An `export * from` whose module is read.
struct StarModule {
    remote: Arc<ModuleExports>,
    type_only: bool,
}

pub struct ImportExport {
    entries: Vec<Entry<'static>>,
    stars: Vec<Star>,
}

/// Every identifier that a destructuring pattern binds, with the `ESTree` parent of each.
fn pattern_bindings(name: P<Node>, holder: Holder, each: &mut impl FnMut(P<Node>, Holder)) {
    let element_holder = match name.kind() {
        Kind::Identifier => return each(name, holder),
        Kind::ObjectBindingPattern => Holder::Property,
        Kind::ArrayBindingPattern => Holder::ArrayPattern,
        _ => return,
    };
    for &element in name.as_binding_pattern().elements.nodes() {
        // A hole of an array pattern binds nothing.
        if element.kind() != Kind::BindingElement {
            continue;
        }
        let element = element.as_binding_element();
        let Some(bound) = element.name else { continue };
        let holder = if element.dot_dot_dot_token().is_some() {
            Holder::RestElement
        } else if element.initializer().is_some() {
            Holder::AssignmentPattern
        } else {
            element_holder
        };
        pattern_bindings(bound, holder, each);
    }
}

/// The block of a namespace: the one of `C` for `namespace A.B.C {}`, which `ESTree` has as one declaration.
fn namespace_block(declaration: P<Node>) -> Option<P<Node>> {
    let mut body = declaration.body()?;
    while body.kind() == Kind::ModuleDeclaration {
        body = body.body()?;
    }
    Some(body)
}

/// True for a namespace that declares only types, which exports no value.
fn is_type_only_namespace(declaration: P<Node>) -> bool {
    let Some(block) = namespace_block(declaration) else {
        return false;
    };
    block.statements().iter().all(|&statement| match statement.kind() {
        Kind::InterfaceDeclaration | Kind::TypeAliasDeclaration => true,
        Kind::ModuleDeclaration => is_type_only_namespace(statement),
        Kind::ImportEqualsDeclaration => {
            let reference = statement.as_import_equals_declaration().module_reference;
            reference.kind() != Kind::ExternalModuleReference
                && !statement.modifier_flags().contains(ModifierFlags::Export)
        }
        _ => false,
    })
}

/// Adds `site` to `sites`, which holds a node once.
fn insert(sites: &mut Vec<Site>, site: Site) {
    if !sites.iter().any(|known| known.node == site.node) {
        sites.push(site);
    }
}

fn holders(sites: &[Site]) -> Vec<Holder> {
    let mut holders = Vec::with_capacity(sites.len());
    for site in sites {
        if !holders.contains(&site.holder) {
            holders.push(site.holder);
        }
    }
    holders
}

/// Overloads are one export: only the implementation counts, or the first.
fn remove_function_overloads(sites: &mut Vec<Site>) {
    let has_implementation = sites.iter().any(Site::is_function);
    let mut kept_first = has_implementation;
    sites.retain(|site| !site.is_overload() || !std::mem::replace(&mut kept_first, true));
}

/// A namespace merged with a function, a class or an enum of its name.
fn is_namespace_merging(sites: &[Site]) -> bool {
    let holders = holders(sites);
    let others = sites.iter().filter(|site| site.holder != Holder::ModuleDeclaration).count();
    let has_function = holders.contains(&Holder::FunctionDeclaration);
    let has_overload = holders.contains(&Holder::DeclareFunction);
    holders.contains(&Holder::ModuleDeclaration)
        && (holders.len() == 1
            || (holders.len() == 2 && (has_function || has_overload))
            || (holders.len() == 3 && has_function && has_overload)
            || (holders.len() == 2
                && (holders.contains(&Holder::ClassDeclaration)
                    || holders.contains(&Holder::EnumDeclaration))
                && others == 1))
}

/// Among the sites of one kind (value or type), the duplicates to report.
fn find_duplicates(sites: &mut Vec<Site>, duplicates: &mut Vec<Site>) {
    remove_function_overloads(sites);
    if sites.len() <= 1 || is_namespace_merging(sites) {
        return;
    }
    // A namespace next to several declarations it can merge with is not reported: those declarations are.
    let holders = holders(sites);
    let skips_namespaces = [
        Holder::ClassDeclaration,
        Holder::FunctionDeclaration,
        Holder::DeclareFunction,
        Holder::EnumDeclaration,
    ]
    .iter()
    .any(|mergeable| holders.contains(mergeable));
    for &site in sites.iter() {
        if !(skips_namespaces && site.holder == Holder::ModuleDeclaration) {
            insert(duplicates, site);
        }
    }
}

/// The range of the `ESTree` node that `site` stands for.
fn range_of(site: &Site, context: &RuleContext<'_>) -> Range {
    let mut range = trim_node_range(context.source_file, site.node);
    match site.form {
        // An `export default` starts at `export`, after the decorators of a class.
        Form::Default(_) => {
            let modifiers = site.node.modifier_nodes();
            if let Some(&export) =
                modifiers.iter().find(|modifier| modifier.kind() == Kind::ExportKeyword)
            {
                range.pos = trim_node_range(context.source_file, export).pos;
            }
        }
        // The identifier of an annotated variable has its type: `a: number`.
        Form::Name if site.holder == Holder::VariableDeclarator => {
            if let Some(annotation) =
                site.node.parent().and_then(|declaration| declaration.type_node())
            {
                range.end = annotation.end().expect_source("parsed lint node");
            }
        }
        _ => {}
    }
    range
}

impl ImportExport {
    pub fn new(options: &[Value]) -> Result<Self, String> {
        match options.first() {
            None => Ok(Self { entries: Vec::new(), stars: Vec::new() }),
            Some(Value::Object(members)) if !members.is_empty() => Err(format!(
                "js/import-export does not accept options: remove the option `{}`",
                members.keys().collect::<Vec<_>>().join("`, `")
            )),
            Some(_) => Err("js/import-export does not accept options: remove its options".into()),
        }
    }

    fn add(
        &mut self,
        statement: P<Node>,
        name: &'static str,
        side: Side,
        node: P<Node>,
        holder: Holder,
    ) {
        self.entries.push(Entry {
            scope: scope_of(statement),
            name,
            side,
            site: Site { node, holder, form: Form::Name },
        });
    }

    fn add_default(&mut self, statement: P<Node>, exported: DefaultExport) {
        let holder =
            Holder::Statements(statement.parent().map_or(Kind::Unknown, |parent| parent.kind()));
        self.entries.push(Entry {
            scope: scope_of(statement),
            name: "default",
            side: Side::Value,
            site: Site { node: statement, holder, form: Form::Default(exported) },
        });
    }

    /// `export * from`, `export { a, b as c }` and `export { a } from`.
    fn export_declaration(&mut self, statement: P<Node>) {
        let declaration = statement.as_export_declaration();
        let star = match declaration.export_clause {
            None => true,
            // `export * as name` exports one name, which no other export may have. The JS rule reads the name of an
            // identifier only, and takes `export * as 'name'` for an `export *`.
            Some(clause) if clause.kind() == Kind::NamespaceExport => {
                clause.as_namespace_export().name().kind() == Kind::StringLiteral
            }
            Some(clause) => {
                for &specifier in clause.as_named_exports().elements.nodes() {
                    let exported = specifier.as_export_specifier();
                    let is_type = declaration.is_type_only || exported.is_type_only;
                    self.add(
                        statement,
                        exported.name.text(),
                        if is_type { Side::Type } else { Side::Value },
                        exported.name,
                        Holder::ExportSpecifier,
                    );
                }
                false
            }
        };
        if star
            && let Some(source) =
                declaration.module_specifier.filter(|source| source.kind() == Kind::StringLiteral)
        {
            self.stars.push(Star {
                node: statement,
                scope: scope_of(statement),
                source,
                type_only: declaration.is_type_only,
            });
        }
    }

    /// A declaration with `export`: the `ESTree` `ExportNamedDeclaration` or `ExportDefaultDeclaration`.
    fn exported_declaration(&mut self, statement: P<Node>) {
        let flags = statement.modifier_flags();
        // The `B` of `namespace A.B {}` is a declaration in the body of `A`, exported without the keyword.
        if !flags.contains(ModifierFlags::Export)
            || statement.parent().is_some_and(|parent| parent.kind() == Kind::ModuleDeclaration)
        {
            return;
        }
        let kind = statement.kind();
        let has_body = || statement.body().is_some();
        if flags.contains(ModifierFlags::Default) {
            let exported = match kind {
                Kind::FunctionDeclaration if has_body() => DefaultExport::Function,
                Kind::FunctionDeclaration => DefaultExport::Overload,
                _ => DefaultExport::Other,
            };
            return self.add_default(statement, exported);
        }
        if kind == Kind::VariableStatement {
            let list = statement.as_variable_statement().declaration_list;
            for &declaration in list.as_variable_declaration_list().declarations.nodes() {
                pattern_bindings(
                    declaration.as_variable_declaration().name,
                    Holder::VariableDeclarator,
                    &mut |identifier, holder| {
                        self.add(statement, identifier.text(), Side::Value, identifier, holder);
                    },
                );
            }
            return;
        }
        let (side, holder) = match kind {
            Kind::FunctionDeclaration if has_body() => (Side::Value, Holder::FunctionDeclaration),
            Kind::FunctionDeclaration => (Side::Value, Holder::DeclareFunction),
            Kind::ClassDeclaration => (Side::Value, Holder::ClassDeclaration),
            Kind::EnumDeclaration => (Side::Value, Holder::EnumDeclaration),
            Kind::InterfaceDeclaration => (Side::Type, Holder::InterfaceDeclaration),
            Kind::TypeAliasDeclaration => (Side::Type, Holder::TypeAliasDeclaration),
            Kind::ImportEqualsDeclaration => (Side::Value, Holder::ImportEqualsDeclaration),
            _ if is_type_only_namespace(statement) => return,
            _ => (Side::Value, Holder::ModuleDeclaration),
        };
        // `A.B` and `'module'` are no identifier. The JS rule gives them all one name, which only namespaces
        // have: they merge, and none is reported.
        if let Some(name) = statement.name().filter(|name| name.kind() == Kind::Identifier)
            && !(kind == Kind::ModuleDeclaration
                && statement.body().is_some_and(|body| body.kind() == Kind::ModuleDeclaration))
        {
            self.add(statement, name.text(), side, name, holder);
        }
    }

    /// Reads the module of each `export *` and reports the duplicates of each name: the `ESTree` `Program:exit`.
    fn finish(&mut self, context: &mut RuleContext<'_>) {
        if self.stars.is_empty() && self.entries.len() < 2 {
            return;
        }
        let modules = Modules::new(context.program);
        let source_file = context.source_file;
        let importer = source_file.file_name();
        let mut star_modules = Vec::with_capacity(self.stars.len());
        let mut star_names = Vec::with_capacity(self.stars.len());
        for star in &self.stars {
            let specifier = star.source.text();
            let Some(remote) = modules.resolve_module_exports(specifier, importer) else {
                star_modules.push(None);
                continue;
            };
            let names = modules.export_names(&remote);
            // A module whose own `export *` cannot be read may still export names.
            if names.is_complete && names.names.iter().all(|name| name == "default") {
                context.report_node(
                    star.source,
                    RuleMessage {
                        id: "noNamedExports".into(),
                        description: format!("No named exports found in module '{specifier}'."),
                        help: String::new(),
                    },
                );
            }
            star_names.push((star_modules.len(), names));
            star_modules.push(Some(StarModule { remote, type_only: star.type_only }));
        }
        let mut entries: Vec<Entry<'_>> = std::mem::take(&mut self.entries);
        for (index, names) in &star_names {
            let star = &self.stars[*index];
            let holder = Holder::Statements(
                star.node.parent().map_or(Kind::Unknown, |parent| parent.kind()),
            );
            let site = Site { node: star.node, holder, form: Form::Star(*index) };
            entries.extend(
                names.names.iter().filter(|name| *name != "default").map(|name| Entry {
                    scope: star.scope,
                    name,
                    side: Side::Star,
                    site,
                }),
            );
        }
        // The exports of one name of one scope, side by side and in the order of the file.
        entries.sort_by(|first, second| first.key().cmp(&second.key()));
        for group in entries.chunk_by(|first, second| first.key() == second.key()) {
            if group.len() > 1 {
                report_duplicates(group, &star_modules, &modules, context);
            }
        }
    }
}

/// The namespace block that `statement` exports from, else the module.
fn scope_of(statement: P<Node>) -> Option<P<Node>> {
    statement.parent().filter(|parent| parent.kind() == Kind::ModuleBlock)
}

/// Reports the exports of one name that are duplicates of one another.
fn report_duplicates(
    group: &[Entry<'_>],
    star_modules: &[Option<StarModule>],
    modules: &Modules,
    context: &mut RuleContext<'_>,
) {
    let name = group[0].name;
    let sites = |side| group.iter().filter(move |entry| entry.side == side).map(|entry| entry.site);
    let mut value: Vec<Site> = sites(Side::Value).collect();
    let mut types: Vec<Site> = sites(Side::Type).collect();
    // Each `export *` that brings the name in counts as a value, a type or both.
    for site in sites(Side::Star) {
        let Form::Star(index) = site.form else { continue };
        let Some(star) = &star_modules[index] else { continue };
        let kind = modules.export_kind(&star.remote, name, star.type_only);
        if kind.is_some_and(|kind| kind != ExportKind::Type) {
            insert(&mut value, site);
        }
        if matches!(kind, Some(ExportKind::Type | ExportKind::Both)) {
            insert(&mut types, site);
        }
    }
    if types.iter().any(Site::is_star) {
        // A class or an enum of the file is also a type, which a star may clash with.
        for &site in &value {
            if matches!(site.holder, Holder::ClassDeclaration | Holder::EnumDeclaration) {
                insert(&mut types, site);
            }
        }
    }
    if value.iter().any(|site| !site.is_star() && !site.is_overload()) {
        // `export type { A }` next to a value `A` exports the same binding.
        for &site in &types {
            if site.holder == Holder::ExportSpecifier {
                insert(&mut value, site);
            }
        }
    }
    let mut duplicates = Vec::new();
    find_duplicates(&mut value, &mut duplicates);
    find_duplicates(&mut types, &mut duplicates);
    for site in &duplicates {
        let description = if name == "default" {
            "Multiple default exports.".into()
        } else {
            format!("Multiple exports of name '{name}'.")
        };
        context.report_range(
            range_of(site, context),
            RuleMessage { id: "duplicateExport".into(), description, help: String::new() },
        );
    }
}

impl Rule for ImportExport {
    fn listens_to(&self, listener: Listener) -> bool {
        matches!(
            listener,
            Listener::Enter(
                Kind::ExportDeclaration
                    | Kind::ExportAssignment
                    | Kind::VariableStatement
                    | Kind::FunctionDeclaration
                    | Kind::ClassDeclaration
                    | Kind::InterfaceDeclaration
                    | Kind::TypeAliasDeclaration
                    | Kind::EnumDeclaration
                    | Kind::ModuleDeclaration
                    | Kind::ImportEqualsDeclaration
                    // The last node of the file: what the JS rule does on `Program:exit` is done there.
                    | Kind::EndOfFile
            )
        )
    }

    fn visit(&mut self, _: Listener, node: P<Node>, context: &mut RuleContext<'_>) {
        // A declaration that the parser makes from the `JSDoc` of a JavaScript file is not in the text.
        if node.flags().intersects(NodeFlags::Reparsed) {
            return;
        }
        match node.kind() {
            Kind::EndOfFile => self.finish(context),
            Kind::ExportDeclaration => self.export_declaration(node),
            Kind::ExportAssignment => {
                // `export =` is no export of a name.
                if !node.as_export_assignment().is_export_equals {
                    self.add_default(node, DefaultExport::Other);
                }
            }
            _ => self.exported_declaration(node),
        }
    }
}
