//! Module resolution as `eslint-plugin-import-x` did it with its default resolver, which `js/import-export` was
//! written against: Node's algorithm with the extensions `.mjs`, `.cjs`, `.js`, `.json` and `.node` (no `.ts`), the
//! conditions `import`, `require` and `default`, the main fields `module` and `main`, and without following
//! symbolic links: a workspace package resolves under `node_modules`, not to its real directory.
//!
//! Port of `src/rules/import/resolve.ts` of `revolugo-lint-plugins`. The program's own resolution is not used: it
//! finds `./a.ts` for `./a` and reads `paths`, `types` conditions and declaration files, where the JS rule finds
//! nothing or another file, and the rule reports what the JS rule reports.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Instant;

use tsrs_core::collections::SyncMap;
use tsrs_core::json::{self, Value};
use tsrs_vfs::FS;

const EXTENSIONS: [&str; 5] = [".mjs", ".cjs", ".js", ".json", ".node"];
const MAIN_FIELDS: [&str; 2] = ["module", "main"];

/// `module.builtinModules` of the Node that runs the JS rule (26.13): a name without `node:` is a builtin with and
/// without the prefix, a name with it only as written.
const BUILTIN_MODULES: [&str; 67] = [
    "_http_agent",
    "_http_client",
    "_http_common",
    "_http_incoming",
    "_http_outgoing",
    "_http_server",
    "_tls_common",
    "_tls_wrap",
    "assert",
    "assert/strict",
    "async_hooks",
    "buffer",
    "child_process",
    "cluster",
    "console",
    "constants",
    "crypto",
    "dgram",
    "diagnostics_channel",
    "dns",
    "dns/promises",
    "domain",
    "events",
    "fs",
    "fs/promises",
    "http",
    "http2",
    "https",
    "inspector",
    "inspector/promises",
    "module",
    "net",
    "os",
    "path",
    "path/posix",
    "path/win32",
    "perf_hooks",
    "process",
    "punycode",
    "querystring",
    "readline",
    "readline/promises",
    "repl",
    "stream",
    "stream/consumers",
    "stream/promises",
    "stream/web",
    "string_decoder",
    "sys",
    "timers",
    "timers/promises",
    "tls",
    "trace_events",
    "tty",
    "url",
    "util",
    "util/types",
    "v8",
    "vm",
    "wasi",
    "worker_threads",
    "zlib",
    "node:ffi",
    "node:sea",
    "node:sqlite",
    "node:test",
    "node:test/reporters",
];

/// How long what was read from the disk is trusted, as in the JS rule: a lint run reads each entry once.
const CACHE_LIFETIME_MS: u64 = 30_000;

/// Ends a resolution where Node's algorithm throws rather than moves on.
struct Failure;

type Resolution<T> = Result<T, Failure>;

/// The content of a JSON file.
#[derive(Clone)]
enum JsonFile {
    Missing,
    Invalid,
    Parsed(Arc<Value>),
}

/// What every thread has resolved and read. A key is resolved by any thread that misses it: two threads that miss
/// the same key store the same value.
struct Caches {
    json_files: SyncMap<String, JsonFile>,
    /// By `directory\0specifier`.
    resolutions: SyncMap<String, Option<String>>,
    started: Instant,
    /// Milliseconds after `started`, plus one: zero before the first resolution.
    filled_at: AtomicU64,
}

static CACHES: LazyLock<Caches> = LazyLock::new(|| Caches {
    json_files: SyncMap::default(),
    resolutions: SyncMap::default(),
    started: Instant::now(),
    filled_at: AtomicU64::new(0),
});

fn expire_caches() {
    let caches = &*CACHES;
    let now = u64::try_from(caches.started.elapsed().as_millis()).unwrap_or(u64::MAX - 1) + 1;
    // Relaxed: the time is the only state. The thread whose exchange succeeds clears the caches, which lock
    // themselves.
    let filled_at = caches.filled_at.load(Ordering::Relaxed);
    if (filled_at == 0 || now - filled_at > CACHE_LIFETIME_MS)
        // Relaxed, as the load above.
        && caches
            .filled_at
            .compare_exchange(filled_at, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        caches.json_files.clear();
        caches.resolutions.clear();
    }
}

struct PackageJson {
    directory: String,
    json: Arc<Value>,
}

impl PackageJson {
    /// A member of the `package.json`: none of an array, which JavaScript reads as an object without these keys.
    fn field(&self, key: &str) -> Option<&Value> {
        match &*self.json {
            Value::Object(members) => members.get(key),
            _ => None,
        }
    }
}

/// A specifier matched against an `imports` or `exports` field.
struct Match<'a> {
    key: &'a str,
    /// What `*` stands for, when the key is a pattern.
    pattern: Option<&'a str>,
}

struct PackageRequest<'a> {
    name: &'a str,
    specifier: &'a str,
    /// Empty, or the part of the specifier after the package name.
    subpath: &'a str,
}

struct TargetContext<'a> {
    matched: Match<'a>,
    package_directory: &'a str,
    /// Set for a target of `imports`, which may name a package.
    packages: bool,
}

/// JavaScript's `typeof value === 'object' && value !== null`.
fn is_record(value: &Value) -> bool {
    matches!(value, Value::Object(_) | Value::Array(_))
}

/// The keys of a record as `Object.keys` gives them. An array has its indices, which no caller matches.
fn keys(value: &Value) -> impl Iterator<Item = &str> {
    let members = match value {
        Value::Object(members) => Some(members),
        _ => None,
    };
    members.into_iter().flat_map(tsrs_core::json::JsonMap::keys)
}

fn member<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(members) => members.get(key),
        _ => None,
    }
}

/// `path.posix.normalize`, with (`path.join`) or without (`path.resolve`) the trailing separator of `path`.
fn normalize(path: &str, keep_trailing_separator: bool) -> String {
    let absolute = path.starts_with('/');
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.last().is_some_and(|last| *last != "..") {
                    segments.pop();
                } else if !absolute {
                    segments.push("..");
                }
            }
            _ => segments.push(segment),
        }
    }
    let mut normalized = String::with_capacity(path.len());
    if absolute {
        normalized.push('/');
    }
    normalized.push_str(&segments.join("/"));
    if normalized.is_empty() {
        normalized.push('.');
    }
    if keep_trailing_separator && path.ends_with('/') && !normalized.ends_with('/') {
        normalized.push('/');
    }
    normalized
}

/// `path.join(directory, segment)`.
fn join(directory: &str, segment: &str) -> String {
    if segment.is_empty() {
        return normalize(directory, true);
    }
    normalize(&format!("{directory}/{segment}"), true)
}

/// `path.resolve(directory, segment)`, for an absolute `directory`.
fn resolve(directory: &str, segment: &str) -> String {
    if segment.starts_with('/') {
        return normalize(segment, false);
    }
    normalize(&format!("{directory}/{segment}"), false)
}

/// `path.posix.dirname`.
pub(super) fn dirname(path: &str) -> &str {
    let bytes = path.as_bytes();
    let mut end = None;
    let mut matched_separator = true;
    for index in (1..bytes.len()).rev() {
        if bytes[index] == b'/' {
            if !matched_separator {
                end = Some(index);
                break;
            }
        } else {
            matched_separator = false;
        }
    }
    match end {
        None if path.starts_with('/') => "/",
        None => ".",
        Some(1) if path.starts_with('/') => "//",
        Some(end) => &path[..end],
    }
}

/// `path.posix.extname`.
pub(super) fn extname(path: &str) -> &str {
    let name = path.trim_end_matches('/');
    let name = name.rsplit_once('/').map_or(name, |(_, name)| name);
    match name.rfind('.') {
        Some(dot) if dot > 0 && name != ".." => &name[dot..],
        _ => "",
    }
}

/// Negative when `first` is the more specific key, as Node orders patterns.
fn compare_pattern_keys(first: &str, second: &str) -> i64 {
    if first.is_empty() {
        return 1;
    }
    let length = |key: &str| i64::try_from(key.encode_utf16().count()).unwrap_or(i64::MAX);
    let base_length =
        |key: &str| key.find('*').map_or_else(|| length(key), |star| length(&key[..=star]));
    if base_length(first) != base_length(second) {
        return base_length(second) - base_length(first);
    }
    if !first.contains('*') {
        return 1;
    }
    if !second.contains('*') {
        return -1;
    }
    length(second) - length(first)
}

fn match_pattern<'a>(key: &'a str, pattern: &str) -> Option<&'a str> {
    if !pattern.starts_with("./") && !pattern.starts_with('#') {
        return None;
    }
    let Some(star) = pattern.find('*') else {
        return if pattern.ends_with('/') { key.strip_prefix(pattern) } else { None };
    };
    let base = &pattern[..star];
    let trailer = &pattern[star + 1..];
    let rest = key.strip_prefix(base)?;
    if trailer.contains('*') {
        return None;
    }
    if trailer.is_empty() {
        return Some(rest);
    }
    if key.encode_utf16().count() < pattern.encode_utf16().count() {
        return None;
    }
    rest.strip_suffix(trailer)
}

fn expand_target(target: &str, matched: &Match<'_>) -> Resolution<String> {
    let Some(pattern) = matched.pattern else {
        return Ok(target.into());
    };
    if matched.key.contains('*') || target.contains('*') {
        return Ok(target.replace('*', pattern));
    }
    if matched.key.ends_with('/') && target.ends_with('/') {
        return Ok(format!("{target}{pattern}"));
    }
    Err(Failure)
}

fn is_invalid_target(target: &str) -> bool {
    target.split('/').any(|segment| segment == ".." || segment.eq_ignore_ascii_case("node_modules"))
}

fn split_package_specifier(specifier: &str) -> PackageRequest<'_> {
    let mut separator = specifier.find('/');
    if specifier.starts_with('@')
        && let Some(scope_end) = separator
    {
        separator = specifier[scope_end + 1..].find('/').map(|index| index + scope_end + 1);
    }
    match separator {
        None => PackageRequest { name: specifier, specifier, subpath: "" },
        Some(separator) => PackageRequest {
            name: &specifier[..separator],
            specifier,
            subpath: &specifier[separator..],
        },
    }
}

fn has_mixed_keys(exports: &Value) -> bool {
    let is_subpath = |key: &str| key.starts_with('.') || key.starts_with('#');
    keys(exports).any(is_subpath) && keys(exports).any(|key| !is_subpath(key))
}

/// The main export, when `exports` is not a map of subpaths.
fn main_export(exports: &Value) -> Option<&Value> {
    if !is_record(exports) {
        return Some(exports);
    }
    if let Some(main) = member(exports, ".") {
        return Some(main);
    }
    let has_subpaths = keys(exports).any(|key| key.starts_with("./") || key.starts_with('#'));
    (!has_subpaths).then_some(exports)
}

fn is_relative(specifier: &str) -> bool {
    specifier == "."
        || specifier == ".."
        || specifier.starts_with("./")
        || specifier.starts_with("../")
}

/// True for a Node builtin, as `module.isBuiltin` answers.
fn is_builtin(specifier: &str) -> bool {
    BUILTIN_MODULES.contains(&specifier)
        || specifier
            .strip_prefix("node:")
            .is_some_and(|name| !name.starts_with("node:") && BUILTIN_MODULES.contains(&name))
}

/// A specifier without its query and fragment. A `#` or `?` that starts the specifier belongs to the path.
struct Suffixed<'a> {
    fragment: &'a str,
    path: &'a str,
    query: &'a str,
}

fn split_suffix(specifier: &str) -> Suffixed<'_> {
    let skipped = usize::from(specifier.starts_with(['#', '.', '/']));
    let find = |text: &str, wanted: char| {
        text.get(skipped..).and_then(|rest| rest.find(wanted)).map(|index| index + skipped)
    };
    let fragment_start = find(specifier, '#');
    let before_fragment = fragment_start.map_or(specifier, |start| &specifier[..start]);
    let query_start = find(before_fragment, '?');
    Suffixed {
        fragment: fragment_start.map_or("", |start| &specifier[start..]),
        path: query_start.map_or(before_fragment, |start| &specifier[..start]),
        query: query_start.map_or("", |start| &before_fragment[start..]),
    }
}

/// Resolves the specifiers of one request: the files and directories are those of `fs`.
pub(super) struct Resolver<'a> {
    pub(super) fs: &'a dyn FS,
}

impl Resolver<'_> {
    fn is_file(&self, file: &str) -> bool {
        !file.ends_with('/') && self.fs.stat(file).is_some_and(|info| info.mode.is_regular())
    }

    fn is_directory(&self, file: &str) -> bool {
        let file = if file.len() > 1 { file.trim_end_matches('/') } else { file };
        self.fs.stat(file).is_some_and(|info| info.mode.is_dir())
    }

    fn read_json(&self, file: &str) -> JsonFile {
        let caches = &*CACHES;
        if let Some(json) = caches.json_files.load(file) {
            return json;
        }
        let json = match self.fs.stat(file) {
            None => JsonFile::Missing,
            Some(_) => self
                .fs
                .read_file(file)
                .and_then(|text| json::unmarshal(text.trim_start_matches('\u{feff}')).ok())
                .map_or(JsonFile::Invalid, |value| JsonFile::Parsed(Arc::new(value))),
        };
        caches.json_files.store(file.into(), json.clone());
        json
    }

    fn package_json_in(&self, directory: &str) -> Resolution<Option<PackageJson>> {
        let file = join(directory, "package.json");
        if !self.is_file(&file) {
            return Ok(None);
        }
        match self.read_json(&file) {
            JsonFile::Parsed(json) if is_record(&json) => {
                Ok(Some(PackageJson { directory: directory.into(), json }))
            }
            _ => Err(Failure),
        }
    }

    fn closest_package_json(&self, directory: &str) -> Resolution<Option<PackageJson>> {
        let mut current = directory;
        loop {
            if let Some(found) = self.package_json_in(current)? {
                return Ok(Some(found));
            }
            let parent = dirname(current);
            if parent == current {
                return Ok(None);
            }
            current = parent;
        }
    }

    fn load_as_file(&self, file: &str) -> Option<String> {
        if self.is_file(file) {
            return Some(file.into());
        }
        EXTENSIONS
            .iter()
            .map(|extension| format!("{file}{extension}"))
            .find(|candidate| self.is_file(candidate))
    }

    fn load_index(&self, directory: &str) -> Option<String> {
        self.load_as_file(&join(directory, "index"))
    }

    fn load_as_directory(&self, directory: &str) -> Resolution<Option<String>> {
        let package_json = self.package_json_in(directory)?;
        for field in MAIN_FIELDS {
            if let Some(Value::String(main)) =
                package_json.as_ref().and_then(|package_json| package_json.field(field))
            {
                let target = join(directory, main);
                if let Some(found) = self.load_as_file(&target).or_else(|| self.load_index(&target))
                {
                    return Ok(Some(found));
                }
            }
        }
        Ok(self.load_index(directory))
    }

    /// `.` and a path ending with `/` name a directory, never a file.
    fn load_as_file_or_directory(&self, file: &str, specifier: &str) -> Resolution<Option<String>> {
        if specifier != "."
            && !specifier.ends_with('/')
            && let Some(found) = self.load_as_file(file)
        {
            return Ok(Some(found));
        }
        if self.is_directory(file) { self.load_as_directory(file) } else { Ok(None) }
    }

    /// The file that a target of `imports` or `exports` names. A target may carry a query (`#assets/logo.svg?raw`
    /// matched by a pattern), which is not part of the file name.
    fn resolve_esm_match(&self, target: String) -> Resolution<String> {
        if let Some(found) = self.load_as_file_or_directory(&target, "")? {
            return Ok(found);
        }
        let mut rest = target.as_str();
        while let Some(query) = rest.rfind('?') {
            rest = &rest[..query];
            if self.load_as_file_or_directory(rest, "")?.is_some() {
                return Ok(target);
            }
        }
        Err(Failure)
    }

    fn resolve_string_target(
        &self,
        target: &str,
        context: &TargetContext<'_>,
    ) -> Resolution<String> {
        if target.starts_with("./") {
            let expanded = expand_target(target, &context.matched)?;
            if is_invalid_target(&expanded) {
                return Err(Failure);
            }
            return Ok(resolve(context.package_directory, &expanded));
        }
        if !context.packages || target.starts_with("../") || target.starts_with('/') {
            return Err(Failure);
        }
        let expanded = expand_target(target, &context.matched)?;
        self.resolve_package(context.package_directory, &split_package_specifier(&expanded))
    }

    fn resolve_target(
        &self,
        target: Option<&Value>,
        context: &TargetContext<'_>,
    ) -> Resolution<Option<String>> {
        match target {
            Some(Value::String(target)) => self.resolve_string_target(target, context).map(Some),
            Some(Value::Array(targets)) => {
                if targets.is_empty() {
                    return Err(Failure);
                }
                // The first target that resolves: one that fails is skipped.
                Ok(targets
                    .iter()
                    .find_map(|target| self.resolve_target(Some(target), context).ok().flatten()))
            }
            Some(Value::Object(conditions)) => {
                for (condition, value) in conditions {
                    if matches!(condition, "default" | "import" | "require")
                        && let Some(resolved) = self.resolve_target(Some(value), context)?
                    {
                        return Ok(Some(resolved));
                    }
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// What `key` maps to in an `imports` field, or in the keys of `exports`.
    fn resolve_in_map(
        &self,
        key: &str,
        map: &Value,
        package_directory: &str,
        packages: bool,
    ) -> Resolution<Option<String>> {
        if key.ends_with('/') {
            return Ok(None);
        }
        let context = |matched| TargetContext { matched, package_directory, packages };
        if !key.contains('*')
            && let Some(target) = member(map, key)
        {
            return self.resolve_target(Some(target), &context(Match { key, pattern: None }));
        }
        let mut best: Option<Match<'_>> = None;
        for pattern in keys(map) {
            if let Some(matched) = match_pattern(key, pattern)
                && compare_pattern_keys(best.as_ref().map_or("", |best| best.key), pattern) > 0
            {
                best = Some(Match { key: pattern, pattern: Some(matched) });
            }
        }
        match best {
            None => Ok(None),
            Some(best) => self.resolve_target(member(map, best.key), &context(best)),
        }
    }

    fn resolve_exports(
        &self,
        package_directory: &str,
        subpath: &str,
        exports: &Value,
    ) -> Resolution<String> {
        if is_record(exports) && has_mixed_keys(exports) {
            return Err(Failure);
        }
        let mut resolved = None;
        if subpath == "." {
            resolved = self.resolve_target(
                main_export(exports),
                &TargetContext {
                    matched: Match { key: ".", pattern: None },
                    package_directory,
                    packages: false,
                },
            )?;
        }
        if resolved.is_none() && is_record(exports) {
            resolved = self.resolve_in_map(subpath, exports, package_directory, false)?;
        }
        self.resolve_esm_match(resolved.ok_or(Failure)?)
    }

    fn require_relative(&self, directory: &str, specifier: &str) -> Resolution<String> {
        self.load_as_file_or_directory(&resolve(directory, specifier), specifier)?.ok_or(Failure)
    }

    /// The package `request` names, looked up from `directory` upwards.
    fn resolve_package(&self, directory: &str, request: &PackageRequest<'_>) -> Resolution<String> {
        let subpath = format!(".{}", request.subpath);
        let mut current = directory;
        loop {
            let package_directory = join(&join(current, "node_modules"), request.name);
            if self.is_directory(&package_directory) {
                let package_json = self.package_json_in(&package_directory)?;
                if let Some(exports) =
                    package_json.as_ref().and_then(|package_json| package_json.field("exports"))
                {
                    return self.resolve_exports(&package_directory, &subpath, exports);
                }
                return self.require_relative(&package_directory, &subpath);
            }
            let parent = dirname(current);
            if parent == current {
                return Err(Failure);
            }
            current = parent;
        }
    }

    /// A package that imports itself by its name, through its `exports`.
    fn resolve_self(&self, directory: &str, specifier: &str) -> Resolution<Option<String>> {
        let Some(package_json) = self.closest_package_json(directory)? else {
            return Ok(None);
        };
        let (Some(Value::String(name)), Some(exports)) =
            (package_json.field("name"), package_json.field("exports"))
        else {
            return Ok(None);
        };
        match specifier.strip_prefix(name.as_str()) {
            Some(subpath) if subpath.is_empty() || subpath.starts_with('/') => self
                .resolve_exports(&package_json.directory, &format!(".{subpath}"), exports)
                .map(Some),
            _ => Ok(None),
        }
    }

    fn resolve_in_node_modules(
        &self,
        node_modules: &str,
        request: &PackageRequest<'_>,
    ) -> Resolution<Option<String>> {
        let package_directory = join(node_modules, request.name);
        if self.is_directory(&package_directory) {
            let package_json = self.package_json_in(&package_directory)?;
            if let Some(exports) =
                package_json.as_ref().and_then(|package_json| package_json.field("exports"))
            {
                return self
                    .resolve_exports(&package_directory, &format!(".{}", request.subpath), exports)
                    .map(Some);
            }
        } else if !request.subpath.is_empty() {
            return Ok(None);
        }
        self.load_as_file_or_directory(&join(node_modules, request.specifier), request.specifier)
    }

    fn require_bare(&self, directory: &str, specifier: &str) -> Resolution<String> {
        if let Some(own) = self.resolve_self(directory, specifier)? {
            return Ok(own);
        }
        let request = split_package_specifier(specifier);
        let mut current = directory;
        loop {
            let node_modules = join(current, "node_modules");
            if self.is_directory(&node_modules)
                && let Some(found) = self.resolve_in_node_modules(&node_modules, &request)?
            {
                return Ok(found);
            }
            let parent = dirname(current);
            if parent == current {
                return Err(Failure);
            }
            current = parent;
        }
    }

    /// `#name`, which only the `imports` field of the closest `package.json` maps: the aliases of a bundler
    /// (`#imports`, `#components`) do not resolve.
    fn require_hash(&self, directory: &str, specifier: &str) -> Resolution<String> {
        let package_json = self.closest_package_json(directory)?.ok_or(Failure)?;
        let imports = package_json.field("imports").filter(|imports| is_record(imports));
        let resolved = match imports {
            Some(imports) if specifier != "#" && !specifier.starts_with("#/") => {
                self.resolve_in_map(specifier, imports, &package_json.directory, true)?
            }
            _ => None,
        };
        self.resolve_esm_match(resolved.ok_or(Failure)?)
    }

    fn require_path(&self, directory: &str, specifier: &str) -> Resolution<String> {
        if specifier.starts_with('/') {
            return self.load_as_file_or_directory(specifier, specifier)?.ok_or(Failure);
        }
        if is_relative(specifier) {
            return self.require_relative(directory, specifier);
        }
        if specifier.starts_with('#') {
            self.require_hash(directory, specifier)
        } else {
            self.require_bare(directory, specifier)
        }
    }

    fn resolve_uncached(&self, directory: &str, specifier: &str) -> Resolution<String> {
        let parts = split_suffix(specifier);
        if parts.path.is_empty() {
            return Err(Failure);
        }
        if !parts.fragment.is_empty()
            && parts.query.is_empty()
            // `#` may also be a character of the file name: that reading comes first.
            && let Ok(found) =
                self.require_path(directory, &format!("{}{}", parts.path, parts.fragment))
        {
            return Ok(found);
        }
        let mut resolved = self.require_path(directory, parts.path)?;
        resolved.push_str(parts.query);
        resolved.push_str(parts.fragment);
        Ok(resolved)
    }

    /// The resolved file, once the `package.json` it belongs to is known to be readable: a file under an invalid
    /// `package.json` does not resolve.
    fn resolve_checked(&self, directory: &str, specifier: &str) -> Resolution<String> {
        let resolved = self.resolve_uncached(directory, specifier)?;
        self.closest_package_json(dirname(&resolved))?;
        Ok(resolved)
    }

    /// The file that `specifier` names when imported from `source_file`: none for a Node builtin, a `data:` URL
    /// and a specifier that does not resolve. A query or a fragment stays at the end of the path.
    pub(super) fn resolve_module(&self, specifier: &str, source_file: &str) -> Option<String> {
        if is_builtin(specifier) || specifier.starts_with("data:") {
            return None;
        }
        expire_caches();
        let directory = dirname(source_file);
        let key = format!("{directory}\0{specifier}");
        if let Some(resolved) = CACHES.resolutions.load(&key) {
            return resolved;
        }
        let resolved = self.resolve_checked(directory, specifier).ok();
        CACHES.resolutions.store(key, resolved.clone());
        resolved
    }
}
