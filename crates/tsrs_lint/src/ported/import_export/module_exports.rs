//! What another module exports, read from its source: the names it declares, the ones it re-exports and its
//! `export * from`, which `js/import-export` follows to find a name exported twice.
//!
//! Port of `src/rules/import/module-exports.ts` and `module-parser.ts` of `revolugo-lint-plugins`. The JS rule
//! parses the module with `@typescript-eslint/parser`. Here the statements come from the file that the program has
//! already parsed, when it has the module, and from a parse of the file otherwise (a script of a package, the
//! `<script>` blocks of a Vue component). Both read the same statements, so both give the same exports.
//!
//! The exports of a module do not depend on the program that asks for them, so every checker thread of every
//! program shares one cache of them, by file.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};

use rustc_hash::{FxHashMap, FxHashSet};
use tsrs_ast::{Kind, ModifierFlags, Node, NodeFlags, SourceFile, SourceFileParseOptions};
use tsrs_compiler::Program;
use tsrs_core::P;
use tsrs_core::arena::Region;
use tsrs_core::collections::SyncMap;
use tsrs_core::tspath;
use tsrs_vfs::{FS, osvfs, wrapvfs};

use super::component::script_blocks;
use super::resolve::{Resolver, extname};
use super::{Holder, pattern_bindings};

/// What a name exports: a value, a type, or both (a class, an enum, a value and a type of one name). `Unknown`
/// when the module it comes from cannot be read, which counts as a value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ExportKind {
    Both,
    Type,
    Unknown,
    Value,
}

/// What a declaration is: a plain value, a type, or both.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Declared {
    #[default]
    Value,
    Both,
    Type,
}

/// A name that comes from another module.
#[derive(Clone)]
struct Link {
    /// The name in the other module.
    local: String,
    specifier: String,
    type_only: bool,
}

/// A binding of a module, or what one of its own exports declares.
#[derive(Clone, Default)]
struct Binding {
    /// Set when the binding is an import.
    alias: Option<Link>,
    kind: Declared,
    /// True for `export type { a }` and `import type`.
    type_only: bool,
}

struct StarExport {
    specifier: String,
    type_only: bool,
}

pub(super) struct ModuleExports {
    file: String,
    own: FxHashMap<String, Binding>,
    /// `export { local as name } from 'specifier'`, by exported name.
    reexports: FxHashMap<String, Link>,
    stars: Vec<StarExport>,
    /// The names of the module, with the generation they were read in.
    names: Mutex<Option<(u64, Arc<ExportNames>)>>,
}

/// The names that a module exports, as far as its `export *` can be read.
pub(super) struct ExportNames {
    /// False when an `export *` of the module, or of one it re-exports, names a module that cannot be read: it has
    /// names that are not in `names`.
    pub(super) is_complete: bool,
    pub(super) names: Vec<String>,
}

/// What tells a version of a file from another.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stamp {
    Missing,
    /// The modification time, when the file system has one.
    Modified(Option<SystemTime>),
}

#[derive(Clone)]
struct CacheEntry {
    checked_at: Instant,
    exports: Option<Arc<ModuleExports>>,
    stamp: Stamp,
}

/// How long a file is trusted before its modification time is read again, as in the JS rule.
const RECHECK_AFTER: Duration = Duration::from_secs(1);

/// The exports of the modules read so far, by file. A thread that misses a file reads it: two threads that miss
/// the same file store equal exports.
static MODULES: LazyLock<SyncMap<String, CacheEntry>> = LazyLock::new(SyncMap::default);

/// Incremented when a module read before has changed on the disk: the names read before are read again.
static GENERATION: AtomicU64 = AtomicU64::new(0);

const SCRIPT_EXTENSIONS: [&str; 8] = [".cjs", ".cts", ".js", ".jsx", ".mjs", ".mts", ".ts", ".tsx"];

/// The languages of a `<script>` block that the parser reads.
const SCRIPT_LANGUAGES: [&str; 4] = ["js", "jsx", "ts", "tsx"];

fn union_kind(first: Declared, second: Declared) -> Declared {
    if first == second { first } else { Declared::Both }
}

fn declared_kind(declaration: P<Node>) -> Declared {
    match declaration.kind() {
        Kind::ClassDeclaration | Kind::EnumDeclaration => Declared::Both,
        Kind::InterfaceDeclaration | Kind::TypeAliasDeclaration => Declared::Type,
        _ => Declared::Value,
    }
}

/// How `export` makes a declaration an export of the module.
#[derive(PartialEq, Eq)]
enum Exported {
    No,
    Named,
    Default,
}

/// True for the declarations that `export` wraps in an export statement. An `export import a = b` stays an
/// import.
fn is_exportable(statement: P<Node>) -> bool {
    matches!(
        statement.kind(),
        Kind::VariableStatement
            | Kind::FunctionDeclaration
            | Kind::ClassDeclaration
            | Kind::InterfaceDeclaration
            | Kind::TypeAliasDeclaration
            | Kind::EnumDeclaration
            | Kind::ModuleDeclaration
    )
}

fn exported(statement: P<Node>) -> Exported {
    if !is_exportable(statement) {
        return Exported::No;
    }
    let flags = statement.modifier_flags();
    if !flags.contains(ModifierFlags::Export) {
        Exported::No
    } else if flags.contains(ModifierFlags::Default) {
        Exported::Default
    } else {
        Exported::Named
    }
}

/// The name of a declaration, when it is one identifier: `A.B` and `'module'` name no binding.
fn identifier_name(declaration: P<Node>) -> Option<&'static str> {
    let name = declaration.name().filter(|name| name.kind() == Kind::Identifier)?;
    let is_qualified = declaration.kind() == Kind::ModuleDeclaration
        && declaration.body().is_some_and(|body| body.kind() == Kind::ModuleDeclaration);
    (!is_qualified).then(|| name.text())
}

/// The value of a module specifier. None when it is not a string, which the JS parser refuses.
fn specifier_of(node: Option<P<Node>>) -> Option<&'static str> {
    node.filter(|node| node.kind() == Kind::StringLiteral).map(|node| node.text())
}

/// The identifiers that a variable statement binds.
fn declared_identifiers(statement: P<Node>, each: &mut impl FnMut(&'static str)) {
    let list = statement.as_variable_statement().declaration_list;
    for &declaration in list.as_variable_declaration_list().declarations.nodes() {
        pattern_bindings(
            declaration.as_variable_declaration().name,
            Holder::VariableDeclarator,
            &mut |identifier, _| each(identifier.text()),
        );
    }
}

/// What a declaration adds to the kind of a name; none for a type-only one.
fn contribution(binding: &Binding) -> Option<Declared> {
    if !binding.type_only {
        Some(binding.kind)
    } else if binding.kind == Declared::Value {
        None
    } else {
        Some(Declared::Type)
    }
}

/// The exports that the statements of a file declare, one script at a time.
struct Analysis {
    exports: ModuleExports,
    /// True once a statement makes the file an ES module.
    is_module: bool,
}

impl Analysis {
    fn new(file: &str) -> Self {
        Self {
            exports: ModuleExports {
                file: file.into(),
                own: FxHashMap::default(),
                reexports: FxHashMap::default(),
                stars: Vec::new(),
                names: Mutex::new(None),
            },
            is_module: false,
        }
    }

    /// Adds an export of `name`, merged with the ones the module already has.
    fn add_own(&mut self, name: &str, added: Binding) {
        let Some(previous) = self.exports.own.get_mut(name) else {
            self.exports.own.insert(name.into(), added);
            return;
        };
        let mut kind = Declared::Both;
        if previous.alias.is_none() && added.alias.is_none() {
            kind = match (contribution(previous), contribution(&added)) {
                (None, second) => second.unwrap_or_default(),
                (Some(first), None) => first,
                (Some(first), Some(second)) => union_kind(first, second),
            };
        }
        *previous = Binding { alias: None, kind, type_only: false };
    }

    /// What each top-level name of a script is: an import, a type, a value. None for a script that the JS parser
    /// refuses.
    fn collect_bindings(statements: &[P<Node>]) -> Option<FxHashMap<&'static str, Binding>> {
        let mut bindings: FxHashMap<&'static str, Binding> = FxHashMap::default();
        let declare = |bindings: &mut FxHashMap<&'static str, Binding>, name, kind| {
            let kind = bindings.get(name).map_or(kind, |previous| union_kind(previous.kind, kind));
            bindings.insert(name, Binding { alias: None, kind, type_only: false });
        };
        for &statement in statements {
            if statement.flags().intersects(NodeFlags::Reparsed) {
                continue;
            }
            match statement.kind() {
                Kind::ImportDeclaration => {
                    let declaration = statement.as_import_declaration();
                    let specifier = specifier_of(Some(declaration.module_specifier))?;
                    let Some(clause) = declaration.import_clause else { continue };
                    let type_only = clause.is_type_only();
                    let clause = clause.as_import_clause();
                    let link = |local: &str, type_only| {
                        Some(Link { local: local.into(), specifier: specifier.into(), type_only })
                    };
                    if let Some(name) = clause.name {
                        let alias = link("default", type_only);
                        bindings.insert(name.text(), Binding { alias, ..Binding::default() });
                    }
                    match clause.named_bindings {
                        Some(named) if named.kind() == Kind::NamespaceImport => {
                            let name = named.as_namespace_import().name().text();
                            bindings.insert(name, Binding { type_only, ..Binding::default() });
                        }
                        Some(named) => {
                            for &imported in named.as_named_imports().elements.nodes() {
                                let local = imported.name()?.text();
                                let name =
                                    imported.property_name().map_or(local, |name| name.text());
                                let alias = link(name, type_only || imported.is_type_only());
                                bindings.insert(local, Binding { alias, ..Binding::default() });
                            }
                        }
                        None => {}
                    }
                }
                Kind::ImportEqualsDeclaration => {
                    let declaration = statement.as_import_equals_declaration();
                    if declaration.name.kind() == Kind::Identifier {
                        let type_only = declaration.is_type_only;
                        bindings.insert(
                            declaration.name.text(),
                            Binding { type_only, ..Binding::default() },
                        );
                    }
                }
                Kind::VariableStatement => {
                    declared_identifiers(statement, &mut |name| {
                        declare(&mut bindings, name, Declared::Value);
                    });
                }
                Kind::FunctionDeclaration
                | Kind::ClassDeclaration
                | Kind::InterfaceDeclaration
                | Kind::TypeAliasDeclaration
                | Kind::EnumDeclaration
                | Kind::ModuleDeclaration
                | Kind::NamespaceExportDeclaration => {
                    if let Some(name) = identifier_name(statement) {
                        declare(&mut bindings, name, declared_kind(statement));
                    }
                }
                _ => {}
            }
        }
        Some(bindings)
    }

    fn add_export_declaration(
        &mut self,
        statement: P<Node>,
        bindings: &FxHashMap<&'static str, Binding>,
    ) -> Option<()> {
        let declaration = statement.as_export_declaration();
        let type_only = declaration.is_type_only;
        let Some(clause) = declaration.export_clause else {
            let specifier = specifier_of(declaration.module_specifier)?.into();
            self.exports.stars.push(StarExport { specifier, type_only });
            return Some(());
        };
        if clause.kind() == Kind::NamespaceExport {
            specifier_of(declaration.module_specifier)?;
            let name = clause.as_namespace_export().name().text();
            self.add_own(name, Binding { type_only, ..Binding::default() });
            return Some(());
        }
        let specifier = match declaration.module_specifier {
            None => None,
            specifier => Some(specifier_of(specifier)?),
        };
        for &exported in clause.as_named_exports().elements.nodes() {
            let exported = exported.as_export_specifier();
            let name = exported.name.text();
            let local = exported.property_name.map_or(name, |local| local.text());
            let type_only = type_only || exported.is_type_only;
            match specifier {
                None => {
                    let mut binding = bindings.get(local).cloned().unwrap_or_default();
                    binding.type_only |= type_only;
                    self.add_own(name, binding);
                }
                Some(specifier) => {
                    let link = Link { local: local.into(), specifier: specifier.into(), type_only };
                    self.exports.reexports.insert(name.into(), link);
                }
            }
        }
        Some(())
    }

    /// Adds the exports of one script. None for a script that the JS parser refuses, and for one that uses
    /// `export =`, whose exports are the members of what it assigns.
    fn add_script(&mut self, statements: &[P<Node>]) -> Option<()> {
        let bindings = Self::collect_bindings(statements)?;
        for &statement in statements {
            if statement.flags().intersects(NodeFlags::Reparsed) {
                continue;
            }
            match statement.kind() {
                Kind::ImportDeclaration => self.is_module = true,
                Kind::ExportDeclaration => {
                    self.is_module = true;
                    self.add_export_declaration(statement, &bindings)?;
                }
                Kind::ExportAssignment => {
                    let assignment = statement.as_export_assignment();
                    if assignment.is_export_equals {
                        return None;
                    }
                    self.is_module = true;
                    let exported = tsrs_ast::skip_parentheses(statement.expression()?);
                    let binding = if exported.kind() == Kind::Identifier {
                        bindings.get(exported.text()).cloned().unwrap_or_default()
                    } else {
                        Binding::default()
                    };
                    self.add_own("default", binding);
                }
                _ => match exported(statement) {
                    Exported::No => {}
                    Exported::Default => {
                        self.is_module = true;
                        let kind = declared_kind(statement);
                        self.add_own("default", Binding { kind, ..Binding::default() });
                    }
                    Exported::Named => {
                        self.is_module = true;
                        if statement.kind() == Kind::VariableStatement {
                            declared_identifiers(statement, &mut |name| {
                                self.add_own(name, Binding::default());
                            });
                        } else if let Some(name) = identifier_name(statement) {
                            let kind = declared_kind(statement);
                            self.add_own(name, Binding { kind, ..Binding::default() });
                        }
                    }
                },
            }
        }
        Some(())
    }

    /// The exports read. None for a file without `import` or `export` (a script, or `CommonJS`), unless it is a
    /// component.
    fn finish(mut self, is_component: bool) -> Option<ModuleExports> {
        if is_component {
            // The component itself, which `<script setup>` exports without a statement.
            self.exports.own.entry("default".into()).or_default();
        } else if !self.is_module {
            return None;
        }
        Some(self.exports)
    }
}

/// Parses `text` as the script `name` and gives its statements to `read`. None when the script does not parse.
///
/// The tree is made in a region of its own, freed on return: `read` keeps nothing of it.
fn parse<Read>(
    name: &str,
    text: String,
    read: impl FnOnce(&[P<Node>]) -> Option<Read>,
) -> Option<Read> {
    let region = Region::new_scratch(text.len().saturating_mul(10).clamp(64 << 10, 64 << 20));
    let scope = region.enter_scratch();
    // The compiler host is the way to the parser from this crate: it parses the text that its file system gives.
    let text = Mutex::new(Some(text));
    let fs = wrapvfs::wrap(
        osvfs::fs(),
        wrapvfs::Replacements {
            read_file: Some(Box::new(move |_| text.lock().ok()?.take())),
            ..Default::default()
        },
    );
    let host = tsrs_compiler::new_compiler_host("/", Arc::new(fs), "", None, None, None);
    let file = host.get_source_file(SourceFileParseOptions {
        file_name: name.into(),
        path: tspath::to_path(name, "/", true),
        ..Default::default()
    })?;
    let result =
        if file.diagnostics().is_empty() { read(file.as_node().statements()) } else { None };
    drop(scope);
    result
}

fn is_parsable(file: &str) -> bool {
    let extension = extname(file);
    extension == ".vue" || SCRIPT_EXTENSIONS.contains(&extension)
}

fn merge_kinds(first: Option<ExportKind>, second: Option<ExportKind>) -> Option<ExportKind> {
    let (Some(first), Some(second)) = (first, second) else {
        return first.or(second);
    };
    if first == second {
        return Some(second);
    }
    let known = |kind| if kind == ExportKind::Unknown { ExportKind::Value } else { kind };
    Some(if known(first) == known(second) { known(first) } else { ExportKind::Both })
}

/// The modules that the files of a program name, read through the file system of the program.
pub(super) struct Modules {
    program: &'static Program,
}

impl Modules {
    pub(super) fn new(program: &'static Program) -> Self {
        Self { program }
    }

    fn fs(&self) -> &dyn FS {
        self.program.host().fs()
    }

    /// The script `file` as the program has parsed it, when it has the file: by its name, or by its real path for a
    /// file that the resolution reaches through a symbolic link (a workspace package under `node_modules`). A
    /// content-mapped file has another text than the file.
    fn program_file(&self, file: &str) -> Option<P<SourceFile>> {
        let source_file = self.program.get_source_file(file).or_else(|| {
            let real = self.fs().realpath(file);
            (real != file).then(|| self.program.get_source_file(&real)).flatten()
        })?;
        (!source_file.is_content_mapped()).then_some(source_file)
    }

    /// The exports that the statements of `file` declare. None for a file that does not parse, and for one that is
    /// not an ES module.
    fn analyze(&self, file: &str) -> Option<ModuleExports> {
        let mut analysis = Analysis::new(file);
        if extname(file) == ".vue" {
            let source = self.fs().read_file(file)?;
            for block in script_blocks(source.trim_start_matches('\u{feff}')).into_iter().flatten()
            {
                let language = block.lang.unwrap_or("js");
                if !SCRIPT_LANGUAGES.contains(&language) {
                    return None;
                }
                let name = format!("{file}.{language}");
                parse(&name, block.content.into(), |statements| analysis.add_script(statements))?;
            }
            return analysis.finish(true);
        }
        if let Some(source_file) = self.program_file(file) {
            if !source_file.diagnostics().is_empty() {
                return None;
            }
            analysis.add_script(source_file.as_node().statements())?;
        } else {
            let text = self.fs().read_file(file)?;
            parse(file, text, |statements| analysis.add_script(statements))?;
        }
        analysis.finish(false)
    }

    /// The exports of the module in `file`, read once per version of the file. None for a file that is not a
    /// module the parser reads.
    fn read_module_exports(&self, file: &str) -> Option<Arc<ModuleExports>> {
        let now = Instant::now();
        let cached = MODULES.load(file);
        if let Some(cached) = &cached
            && now.duration_since(cached.checked_at) < RECHECK_AFTER
        {
            return cached.exports.clone();
        }
        let stamp =
            self.fs().stat(file).map_or(Stamp::Missing, |info| Stamp::Modified(info.mod_time));
        if let Some(mut cached) = cached {
            if cached.stamp == stamp {
                let exports = cached.exports.clone();
                cached.checked_at = now;
                MODULES.store(file.into(), cached);
                return exports;
            }
            // Relaxed: a counter. A thread that reads the old value reads names that were true a moment ago.
            GENERATION.fetch_add(1, Ordering::Relaxed);
        }
        let exports = if stamp != Stamp::Missing && is_parsable(file) {
            self.analyze(file).map(Arc::new)
        } else {
            None
        };
        MODULES.store(file.into(), CacheEntry { checked_at: now, exports: exports.clone(), stamp });
        exports
    }

    /// The exports of the module that `specifier` names in `importer`.
    pub(super) fn resolve_module_exports(
        &self,
        specifier: &str,
        importer: &str,
    ) -> Option<Arc<ModuleExports>> {
        let file = Resolver { fs: self.fs() }.resolve_module(specifier, importer)?;
        self.read_module_exports(&file)
    }

    /// True when `module` exports `name`, itself or through an `export *`: the names of a module are read once.
    fn has_export(&self, module: &ModuleExports, name: &str) -> bool {
        self.export_names(module).names.binary_search_by(|known| known.as_str().cmp(name)).is_ok()
    }

    fn collect_names(
        &self,
        module: &ModuleExports,
        found: &mut ExportNames,
        seen: &mut FxHashSet<String>,
        listed: &mut FxHashSet<String>,
    ) {
        if !seen.insert(module.file.clone()) {
            return;
        }
        // `export *` does not re-export the default export of its target.
        let is_top_level = seen.len() == 1;
        for name in module.own.keys().chain(module.reexports.keys()) {
            if (is_top_level || name != "default") && listed.insert(name.clone()) {
                found.names.push(name.clone());
            }
        }
        for star in &module.stars {
            match self.resolve_module_exports(&star.specifier, &module.file) {
                None => found.is_complete = false,
                Some(inner) => self.collect_names(&inner, found, seen, listed),
            }
        }
    }

    /// Every name that `module` exports, its own and through `export *`.
    pub(super) fn export_names(&self, module: &ModuleExports) -> Arc<ExportNames> {
        let generation = GENERATION.load(Ordering::Relaxed);
        if let Ok(cached) = module.names.lock()
            && let Some((read_in, found)) = &*cached
            && *read_in == generation
        {
            return Arc::clone(found);
        }
        // Read without the lock: a module that re-exports itself asks for its own names.
        let mut found = ExportNames { is_complete: true, names: Vec::new() };
        self.collect_names(
            module,
            &mut found,
            &mut FxHashSet::default(),
            &mut FxHashSet::default(),
        );
        // Sorted: `has_export` searches them, and a run reports them in the same order as another.
        found.names.sort_unstable();
        let found = Arc::new(found);
        if let Ok(mut cached) = module.names.lock() {
            *cached = Some((generation, Arc::clone(&found)));
        }
        found
    }

    fn follow(
        &self,
        module: &ModuleExports,
        link: &Link,
        type_only: bool,
        seen: &mut FxHashSet<(String, String, bool)>,
    ) -> Option<ExportKind> {
        let type_only = type_only || link.type_only;
        match self.resolve_module_exports(&link.specifier, &module.file) {
            Some(target) if self.has_export(&target, &link.local) => {
                self.resolve_kind(&target, &link.local, type_only, seen)
            }
            _ => Some(if type_only { ExportKind::Type } else { ExportKind::Unknown }),
        }
    }

    fn resolve_kind(
        &self,
        module: &ModuleExports,
        name: &str,
        type_only: bool,
        seen: &mut FxHashSet<(String, String, bool)>,
    ) -> Option<ExportKind> {
        if !seen.insert((module.file.clone(), name.into(), type_only)) {
            return None;
        }
        if let Some(own) = module.own.get(name) {
            let type_only = type_only || own.type_only;
            if let Some(alias) = &own.alias {
                return self.follow(module, alias, type_only, seen);
            }
            return match (type_only, own.kind) {
                (false, Declared::Value) => Some(ExportKind::Value),
                (false, Declared::Both) => Some(ExportKind::Both),
                (true, Declared::Value) => None,
                (_, Declared::Type) | (true, Declared::Both) => Some(ExportKind::Type),
            };
        }
        if let Some(reexport) = module.reexports.get(name) {
            return self.follow(module, reexport, type_only, seen);
        }
        let mut kind = None;
        if name != "default" {
            for star in &module.stars {
                // A module that does not export the name gives it no kind: the modules it re-exports are not
                // walked, which is most of a barrel.
                if let Some(inner) = self.resolve_module_exports(&star.specifier, &module.file)
                    && self.has_export(&inner, name)
                {
                    let type_only = type_only || star.type_only;
                    kind = merge_kinds(kind, self.resolve_kind(&inner, name, type_only, seen));
                }
            }
        }
        kind
    }

    /// What `name` is when imported from `module`: none when the module does not export it that way (a value
    /// through `export type *`).
    pub(super) fn export_kind(
        &self,
        module: &ModuleExports,
        name: &str,
        type_only: bool,
    ) -> Option<ExportKind> {
        self.resolve_kind(module, name, type_only, &mut FxHashSet::default())
    }
}
