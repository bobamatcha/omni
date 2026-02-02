//! TypeScript/TSX language parser using tree-sitter.

use super::LanguageParser;
use crate::types::*;
use anyhow::Result;
use lasso::ThreadedRodeo;
use std::path::{Path, PathBuf};
use tree_sitter::{Language, Node, Tree};

/// TypeScript/TSX source code parser.
pub struct TypeScriptParser {
    language: Language,
    extensions: &'static [&'static str],
}

impl TypeScriptParser {
    pub fn new_typescript() -> Self {
        Self {
            language: tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            extensions: &["ts", "mts", "cts"],
        }
    }

    pub fn new_tsx() -> Self {
        Self {
            language: tree_sitter_typescript::LANGUAGE_TSX.into(),
            extensions: &["tsx"],
        }
    }
}

impl LanguageParser for TypeScriptParser {
    fn language(&self) -> Language {
        self.language.clone()
    }

    fn extensions(&self) -> &[&str] {
        self.extensions
    }

    fn extract_symbols(
        &self,
        tree: &Tree,
        source: &str,
        file: &Path,
        interner: &ThreadedRodeo,
    ) -> Result<Vec<SymbolDef>> {
        let bytes = source.as_bytes();
        let root = tree.root_node();
        let mut symbols = Vec::new();
        let mut scope_stack = vec![file_scope_for(file)];

        walk_ts_symbols(root, bytes, file, &mut scope_stack, interner, &mut symbols);

        Ok(symbols)
    }

    fn extract_calls(
        &self,
        tree: &Tree,
        source: &str,
        file: &Path,
        interner: &ThreadedRodeo,
    ) -> Result<Vec<CallEdge>> {
        let bytes = source.as_bytes();
        let root = tree.root_node();
        let mut calls = Vec::new();
        let mut scope_stack = vec![file_scope_for(file)];
        let mut fn_stack = Vec::<String>::new();

        walk_ts_calls(
            root,
            bytes,
            file,
            &mut scope_stack,
            &mut fn_stack,
            interner,
            &mut calls,
        );

        Ok(calls)
    }

    fn extract_imports(&self, tree: &Tree, source: &str, file: &Path) -> Result<Vec<ImportInfo>> {
        let bytes = source.as_bytes();
        let root = tree.root_node();
        let mut imports = Vec::new();

        walk_ts_imports(root, bytes, file, &mut imports);

        Ok(imports)
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

fn location_for(node: Node, file: &Path) -> Location {
    let start = node.start_position();
    let end = node.end_position();
    Location::new(file.to_path_buf(), node.start_byte(), node.end_byte()).with_positions(
        start.row + 1,
        start.column + 1,
        end.row + 1,
        end.column + 1,
    )
}

fn text_of(bytes: &[u8], node: Node) -> Option<String> {
    std::str::from_utf8(&bytes[node.start_byte()..node.end_byte()])
        .ok()
        .map(|s| s.to_string())
}

fn strip_quotes(s: &str) -> String {
    s.trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .to_string()
}

fn file_scope_for(file: &Path) -> String {
    let root = find_workspace_root(file).unwrap_or_else(|| {
        file.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("/"))
    });
    let rel = file.strip_prefix(&root).unwrap_or(file);
    let mut rel_str = rel.to_string_lossy().to_string();
    if rel_str.contains('\\') {
        rel_str = rel_str.replace('\\', "/");
    }
    format!("file:{rel_str}")
}

fn find_workspace_root(file: &Path) -> Option<PathBuf> {
    const MARKERS: [&str; 6] = [
        "package.json",
        "tsconfig.json",
        "pnpm-workspace.yaml",
        "yarn.lock",
        "Cargo.toml",
        ".git",
    ];
    let mut current = file.parent();
    while let Some(dir) = current {
        for marker in MARKERS {
            let candidate = dir.join(marker);
            if marker == ".git" {
                if candidate.is_dir() {
                    return Some(dir.to_path_buf());
                }
            } else if candidate.is_file() {
                return Some(dir.to_path_buf());
            }
        }
        current = dir.parent();
    }
    None
}

fn first_identifier(bytes: &[u8], node: Node) -> Option<String> {
    if node.kind() == "identifier" || node.kind() == "property_identifier" {
        return text_of(bytes, node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(name) = first_identifier(bytes, child) {
            return Some(name);
        }
    }
    None
}

fn last_identifier(bytes: &[u8], node: Node) -> Option<String> {
    let text = text_of(bytes, node)?;
    let mut best = None;
    for part in text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        if !part.is_empty() {
            best = Some(part);
        }
    }
    best.map(|s| s.to_string())
}

fn string_literal_value(bytes: &[u8], node: Node) -> Option<String> {
    let text = text_of(bytes, node)?;
    if text.starts_with('"')
        || text.starts_with('\'')
        || text.starts_with('`') && text.ends_with(text.chars().next().unwrap())
    {
        return Some(strip_quotes(&text));
    }
    None
}

fn extract_callee_name(bytes: &[u8], node: Node) -> Option<String> {
    match node.kind() {
        "identifier" | "property_identifier" => text_of(bytes, node),
        "member_expression" => {
            if let Some(property) = node.child_by_field_name("property") {
                if let Some(name) = string_literal_value(bytes, property) {
                    return Some(name);
                }
                if let Some(name) = text_of(bytes, property) {
                    return Some(name);
                }
            }
            last_identifier(bytes, node)
        }
        "subscript_expression" => {
            if let Some(index) = node.child_by_field_name("index") {
                if let Some(name) = string_literal_value(bytes, index) {
                    return Some(name);
                }
                if let Some(name) = text_of(bytes, index) {
                    return Some(name);
                }
            }
            last_identifier(bytes, node)
        }
        "optional_chain" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if let Some(name) = extract_callee_name(bytes, child) {
                    return Some(name);
                }
            }
            last_identifier(bytes, node)
        }
        _ => last_identifier(bytes, node),
    }
}

fn make_scoped_name(scope_stack: &[String], name: &str) -> String {
    let mut full = scope_stack.join("::");
    if !full.is_empty() {
        full.push_str("::");
    }
    full.push_str(name);
    full
}

fn add_symbol(
    symbols: &mut Vec<SymbolDef>,
    interner: &ThreadedRodeo,
    scope_stack: &[String],
    name: &str,
    kind: SymbolKind,
    file: &Path,
    node: Node,
) {
    let scoped_name = make_scoped_name(scope_stack, name);
    symbols.push(SymbolDef {
        name: interner.get_or_intern(name),
        scoped_name: interner.get_or_intern(&scoped_name),
        kind,
        location: location_for(node, file),
        signature: None,
        visibility: Visibility::Private,
        attributes: Vec::new(),
        doc_comment: None,
        parent: None,
    });
}

fn walk_ts_symbols(
    node: Node,
    bytes: &[u8],
    file: &Path,
    scope_stack: &mut Vec<String>,
    interner: &ThreadedRodeo,
    symbols: &mut Vec<SymbolDef>,
) {
    match node.kind() {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = text_of(bytes, name_node) {
                    add_symbol(
                        symbols,
                        interner,
                        scope_stack,
                        &name,
                        SymbolKind::Function,
                        file,
                        node,
                    );
                    scope_stack.push(name);
                    walk_children(node, bytes, file, scope_stack, interner, symbols, walk_ts_symbols);
                    scope_stack.pop();
                    return;
                }
            }
        }
        "method_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = text_of(bytes, name_node) {
                    add_symbol(
                        symbols,
                        interner,
                        scope_stack,
                        &name,
                        SymbolKind::Method,
                        file,
                        node,
                    );
                    scope_stack.push(name);
                    walk_children(node, bytes, file, scope_stack, interner, symbols, walk_ts_symbols);
                    scope_stack.pop();
                    return;
                }
            }
        }
        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = text_of(bytes, name_node) {
                    add_symbol(
                        symbols,
                        interner,
                        scope_stack,
                        &name,
                        SymbolKind::Struct,
                        file,
                        node,
                    );
                    scope_stack.push(name);
                    walk_children(node, bytes, file, scope_stack, interner, symbols, walk_ts_symbols);
                    scope_stack.pop();
                    return;
                }
            }
        }
        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = text_of(bytes, name_node) {
                    add_symbol(
                        symbols,
                        interner,
                        scope_stack,
                        &name,
                        SymbolKind::Trait,
                        file,
                        node,
                    );
                    scope_stack.push(name);
                    walk_children(node, bytes, file, scope_stack, interner, symbols, walk_ts_symbols);
                    scope_stack.pop();
                    return;
                }
            }
        }
        "type_alias_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = text_of(bytes, name_node) {
                    add_symbol(
                        symbols,
                        interner,
                        scope_stack,
                        &name,
                        SymbolKind::TypeAlias,
                        file,
                        node,
                    );
                }
            }
        }
        "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = text_of(bytes, name_node) {
                    add_symbol(
                        symbols,
                        interner,
                        scope_stack,
                        &name,
                        SymbolKind::Enum,
                        file,
                        node,
                    );
                }
            }
        }
        "variable_declarator" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| text_of(bytes, n));
            let init = node.child_by_field_name("value");
            if let (Some(name), Some(init)) = (name, init) {
                if matches!(init.kind(), "arrow_function" | "function") {
                    add_symbol(
                        symbols,
                        interner,
                        scope_stack,
                        &name,
                        SymbolKind::Function,
                        file,
                        node,
                    );
                }
            }
        }
        _ => {}
    }

    walk_children(node, bytes, file, scope_stack, interner, symbols, walk_ts_symbols);
}

fn walk_ts_calls(
    node: Node,
    bytes: &[u8],
    file: &Path,
    scope_stack: &mut Vec<String>,
    fn_stack: &mut Vec<String>,
    interner: &ThreadedRodeo,
    calls: &mut Vec<CallEdge>,
) {
    match node.kind() {
        "function_declaration" | "method_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = text_of(bytes, name_node) {
                    let scoped = make_scoped_name(scope_stack, &name);
                    scope_stack.push(name.clone());
                    fn_stack.push(scoped);
                    walk_children_calls(node, bytes, file, scope_stack, fn_stack, interner, calls);
                    fn_stack.pop();
                    scope_stack.pop();
                    return;
                }
            }
        }
        "class_declaration" | "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = text_of(bytes, name_node) {
                    scope_stack.push(name);
                    walk_children_calls(node, bytes, file, scope_stack, fn_stack, interner, calls);
                    scope_stack.pop();
                    return;
                }
            }
        }
        "variable_declarator" => {
            let name = node
                .child_by_field_name("name")
                .and_then(|n| text_of(bytes, n));
            let init = node.child_by_field_name("value");
            if let (Some(name), Some(init)) = (name, init) {
                if matches!(init.kind(), "arrow_function" | "function") {
                    let scoped = make_scoped_name(scope_stack, &name);
                    fn_stack.push(scoped);
                    walk_children_calls(node, bytes, file, scope_stack, fn_stack, interner, calls);
                    fn_stack.pop();
                    return;
                }
            }
        }
        "call_expression" => {
            if let Some(callee_node) = node
                .child_by_field_name("function")
                .or_else(|| node.child_by_field_name("callee"))
            {
                if let Some(callee_name) = extract_callee_name(bytes, callee_node) {
                    let caller_name = fn_stack
                        .last()
                        .cloned()
                        .unwrap_or_else(|| scope_stack.join("::"));
                    let is_method_call = matches!(
                        callee_node.kind(),
                        "member_expression" | "optional_chain" | "subscript_expression"
                    );
                    calls.push(CallEdge {
                        caller: interner.get_or_intern(&caller_name),
                        callee_name,
                        location: location_for(node, file),
                        is_method_call,
                    });
                }
            }
        }
        _ => {}
    }

    walk_children_calls(node, bytes, file, scope_stack, fn_stack, interner, calls);
}

fn walk_ts_imports(node: Node, bytes: &[u8], file: &Path, imports: &mut Vec<ImportInfo>) {
    if node.kind() == "import_statement" {
        let source_node = node.child_by_field_name("source");
        let path = source_node
            .and_then(|n| text_of(bytes, n))
            .map(|s| strip_quotes(&s))
            .unwrap_or_else(|| "".to_string());
        let name = first_identifier(bytes, node).unwrap_or_else(|| path.clone());
        let text = text_of(bytes, node).unwrap_or_default();
        let is_glob = text.contains('*');

        imports.push(ImportInfo {
            path,
            name,
            is_glob,
            location: location_for(node, file),
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_ts_imports(child, bytes, file, imports);
    }
}

fn walk_children(
    node: Node,
    bytes: &[u8],
    file: &Path,
    scope_stack: &mut Vec<String>,
    interner: &ThreadedRodeo,
    symbols: &mut Vec<SymbolDef>,
    f: fn(Node, &[u8], &Path, &mut Vec<String>, &ThreadedRodeo, &mut Vec<SymbolDef>),
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        f(child, bytes, file, scope_stack, interner, symbols);
    }
}

fn walk_children_calls(
    node: Node,
    bytes: &[u8],
    file: &Path,
    scope_stack: &mut Vec<String>,
    fn_stack: &mut Vec<String>,
    interner: &ThreadedRodeo,
    calls: &mut Vec<CallEdge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_ts_calls(child, bytes, file, scope_stack, fn_stack, interner, calls);
    }
}
