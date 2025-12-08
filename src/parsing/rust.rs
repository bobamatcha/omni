//! Rust language parser using tree-sitter.

use super::LanguageParser;
use crate::types::*;
use anyhow::Result;
use lasso::ThreadedRodeo;
use std::path::Path;
use tree_sitter::{Language, Node, Tree};

/// Rust source code parser.
pub struct RustParser {
    // Parser instance is created per-use since it's not Send
}

impl RustParser {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for RustParser {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageParser for RustParser {
    fn language(&self) -> Language {
        tree_sitter_rust::LANGUAGE.into()
    }

    fn extensions(&self) -> &[&str] {
        &["rs"]
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
        let mut module_stack = vec!["crate".to_string()];
        let mut impl_type_stack = Vec::<String>::new();

        walk_rust_symbols(
            root,
            bytes,
            file,
            &mut module_stack,
            &mut impl_type_stack,
            interner,
            &mut symbols,
        );

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
        let mut module_stack = vec!["crate".to_string()];
        let mut impl_type_stack = Vec::<String>::new();
        let mut fn_scope_stack = Vec::<String>::new();

        walk_rust_calls(
            root,
            bytes,
            file,
            &mut module_stack,
            &mut impl_type_stack,
            &mut fn_scope_stack,
            interner,
            &mut calls,
        );

        Ok(calls)
    }

    fn extract_imports(
        &self,
        tree: &Tree,
        source: &str,
        file: &Path,
    ) -> Result<Vec<ImportInfo>> {
        let bytes = source.as_bytes();
        let root = tree.root_node();

        let mut imports = Vec::new();
        walk_rust_imports(root, bytes, file, &mut imports);

        Ok(imports)
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Extract the last identifier from a node's text.
fn last_ident_of(bytes: &[u8], node: Node) -> Option<String> {
    let text = std::str::from_utf8(&bytes[node.start_byte()..node.end_byte()]).ok()?;
    let mut best = None;
    for part in text.split(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        if !part.is_empty() {
            best = Some(part);
        }
    }
    best.map(|s| s.to_string())
}

/// Create a Location from a tree-sitter node.
fn location_for(node: Node, file: &Path) -> Location {
    let s = node.start_position();
    let e = node.end_position();
    Location {
        file: file.to_path_buf(),
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        start_line: s.row,
        start_col: s.column,
        end_line: e.row,
        end_col: e.column,
    }
}

/// Join scope segments into a scoped name.
fn join_scope(seg: &[String]) -> String {
    seg.join("::")
}

/// Get the innermost impl type name if available.
fn current_impl_type(impl_stack: &[String]) -> Option<&str> {
    impl_stack.last().map(|s| s.as_str())
}

/// Extract the type identifier for an impl item.
fn impl_type_ident(bytes: &[u8], impl_node: Node) -> Option<String> {
    if impl_node.kind() != "impl_item" {
        return None;
    }
    if let Some(ty) = impl_node.child_by_field_name("type") {
        let text = std::str::from_utf8(&bytes[ty.start_byte()..ty.end_byte()]).ok()?;
        // Get the first identifier from the type (e.g., "Foo" from "Foo<T>")
        let name = text
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .find(|s| !s.is_empty())?
            .to_string();
        return Some(name);
    }
    None
}

/// Check if a function has #[test] or #[tokio::test] attribute.
fn has_test_attr(bytes: &[u8], fn_node: Node) -> bool {
    let kind = fn_node.kind();
    if kind != "function_item" {
        return false;
    }

    // Walk preceding siblings for attributes
    let mut cur = fn_node.prev_sibling();
    while let Some(sib) = cur {
        let k = sib.kind();
        if k == "attribute_item" {
            if let Ok(text) = std::str::from_utf8(&bytes[sib.start_byte()..sib.end_byte()]) {
                if text.contains("#[test") || text.contains("#[tokio::test") {
                    return true;
                }
            }
            cur = sib.prev_sibling();
            continue;
        }
        break;
    }
    false
}

/// Extract attributes from preceding siblings.
fn extract_attributes(bytes: &[u8], node: Node) -> Vec<String> {
    let mut attrs = Vec::new();
    let mut cur = node.prev_sibling();
    while let Some(sib) = cur {
        if sib.kind() == "attribute_item" {
            if let Ok(text) = std::str::from_utf8(&bytes[sib.start_byte()..sib.end_byte()]) {
                attrs.push(text.trim().to_string());
            }
            cur = sib.prev_sibling();
        } else {
            break;
        }
    }
    attrs.reverse(); // Put them back in original order
    attrs
}

/// Extract doc comments from preceding siblings.
fn extract_doc_comments(bytes: &[u8], node: Node) -> Option<String> {
    let mut doc_lines = Vec::new();
    let mut cur = node.prev_sibling();

    while let Some(sib) = cur {
        let k = sib.kind();
        if k == "line_comment" {
            if let Ok(text) = std::str::from_utf8(&bytes[sib.start_byte()..sib.end_byte()]) {
                if text.starts_with("///") || text.starts_with("//!") {
                    doc_lines.push(text.trim().to_string());
                } else {
                    break;
                }
            }
            cur = sib.prev_sibling();
        } else if k == "attribute_item" {
            // Skip attributes, keep looking
            cur = sib.prev_sibling();
        } else {
            break;
        }
    }

    doc_lines.reverse();
    if doc_lines.is_empty() {
        None
    } else {
        Some(doc_lines.join("\n"))
    }
}

/// Extract visibility from node children.
fn extract_visibility(bytes: &[u8], node: Node) -> Visibility {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            if let Ok(vis_text) = std::str::from_utf8(&bytes[child.start_byte()..child.end_byte()]) {
                let vis = vis_text.trim();
                if vis == "pub" {
                    return Visibility::Public;
                } else if vis.starts_with("pub(crate)") {
                    return Visibility::Crate;
                } else if vis.starts_with("pub(super)") {
                    return Visibility::Super;
                } else if vis.starts_with("pub(in") || vis.starts_with("pub(self)") {
                    return Visibility::Restricted;
                }
            }
        }
    }
    Visibility::Private
}

/// Extract function signature information.
fn extract_signature(bytes: &[u8], fn_node: Node) -> Signature {
    let mut sig = Signature::default();

    // Extract parameters
    if let Some(params_node) = fn_node.child_by_field_name("parameters") {
        let mut cursor = params_node.walk();
        for child in params_node.children(&mut cursor) {
            if child.kind() == "parameter" || child.kind() == "self_parameter" {
                if let Ok(text) = std::str::from_utf8(&bytes[child.start_byte()..child.end_byte()]) {
                    sig.params.push(text.trim().to_string());
                }
            }
        }
    }

    // Extract return type
    if let Some(ret_node) = fn_node.child_by_field_name("return_type") {
        if let Ok(text) = std::str::from_utf8(&bytes[ret_node.start_byte()..ret_node.end_byte()]) {
            sig.return_type = Some(text.trim().to_string());
        }
    }

    // Check for async/unsafe/const modifiers
    let mut cursor = fn_node.walk();
    for child in fn_node.children(&mut cursor) {
        let kind = child.kind();
        if kind == "async" {
            sig.is_async = true;
        } else if kind == "unsafe" {
            sig.is_unsafe = true;
        } else if kind == "const" {
            sig.is_const = true;
        } else if kind == "type_parameters" {
            if let Ok(text) = std::str::from_utf8(&bytes[child.start_byte()..child.end_byte()]) {
                sig.generics = Some(text.trim().to_string());
            }
        } else if kind == "where_clause" {
            if let Ok(text) = std::str::from_utf8(&bytes[child.start_byte()..child.end_byte()]) {
                sig.where_clause = Some(text.trim().to_string());
            }
        }
    }

    sig
}

// ============================================================================
// Symbol Extraction Walker
// ============================================================================

fn walk_rust_symbols(
    node: Node,
    bytes: &[u8],
    file: &Path,
    module_stack: &mut Vec<String>,
    impl_type_stack: &mut Vec<String>,
    interner: &ThreadedRodeo,
    symbols: &mut Vec<SymbolDef>,
) {
    let kind = node.kind();

    // Enter inline module: mod foo { ... }
    let mut entered_mod = false;
    if kind == "mod_item" {
        let has_body = node.child_by_field_name("body").is_some();
        if has_body {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = last_ident_of(bytes, name_node) {
                    module_stack.push(name.clone());
                    entered_mod = true;

                    // Record module as a symbol
                    let scoped = join_scope(module_stack);
                    let symbol = SymbolDef {
                        name: interner.get_or_intern(&name),
                        scoped_name: interner.get_or_intern(&scoped),
                        kind: SymbolKind::Module,
                        location: location_for(node, file),
                        signature: None,
                        visibility: extract_visibility(bytes, node),
                        attributes: extract_attributes(bytes, node),
                        doc_comment: extract_doc_comments(bytes, node),
                        parent: None,
                    };
                    symbols.push(symbol);
                }
            }
        }
    }

    // Enter impl block
    let mut entered_impl = false;
    if kind == "impl_item" {
        if let Some(ty) = impl_type_ident(bytes, node) {
            impl_type_stack.push(ty.clone());
            entered_impl = true;

            // Record impl as a symbol
            let mut scoped = join_scope(module_stack);
            scoped.push_str("::");
            scoped.push_str(&ty);

            let symbol = SymbolDef {
                name: interner.get_or_intern(&ty),
                scoped_name: interner.get_or_intern(&scoped),
                kind: SymbolKind::Impl,
                location: location_for(node, file),
                signature: None,
                visibility: Visibility::Private, // impls don't have visibility
                attributes: extract_attributes(bytes, node),
                doc_comment: extract_doc_comments(bytes, node),
                parent: None,
            };
            symbols.push(symbol);
        } else {
            impl_type_stack.push("_".to_string());
            entered_impl = true;
        }
    }

    // Extract various symbol definitions
    match kind {
        "function_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(fn_name) = last_ident_of(bytes, name_node) {
                    let mut scoped = join_scope(module_stack);

                    // Check if inside impl block
                    let parent = if let Some(ty) = current_impl_type(impl_type_stack) {
                        if !ty.is_empty() && ty != "_" {
                            scoped.push_str("::");
                            scoped.push_str(ty);
                            Some(interner.get_or_intern(ty))
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    scoped.push_str("::");
                    scoped.push_str(&fn_name);

                    let symbol_kind = if parent.is_some() {
                        SymbolKind::Method
                    } else {
                        SymbolKind::Function
                    };

                    let mut attrs = extract_attributes(bytes, node);

                    // Check if this is a test function and add special attribute if so
                    let is_test = has_test_attr(bytes, node);
                    if is_test && !attrs.iter().any(|a| a.contains("test")) {
                        // Ensure test attribute is included
                        attrs.push("#[test]".to_string());
                    }

                    let symbol = SymbolDef {
                        name: interner.get_or_intern(&fn_name),
                        scoped_name: interner.get_or_intern(&scoped),
                        kind: symbol_kind,
                        location: location_for(node, file),
                        signature: Some(extract_signature(bytes, node)),
                        visibility: extract_visibility(bytes, node),
                        attributes: attrs,
                        doc_comment: extract_doc_comments(bytes, node),
                        parent,
                    };
                    symbols.push(symbol);
                }
            }
        }

        "struct_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = last_ident_of(bytes, name_node) {
                    let mut scoped = join_scope(module_stack);
                    scoped.push_str("::");
                    scoped.push_str(&name);

                    let symbol = SymbolDef {
                        name: interner.get_or_intern(&name),
                        scoped_name: interner.get_or_intern(&scoped),
                        kind: SymbolKind::Struct,
                        location: location_for(node, file),
                        signature: None,
                        visibility: extract_visibility(bytes, node),
                        attributes: extract_attributes(bytes, node),
                        doc_comment: extract_doc_comments(bytes, node),
                        parent: None,
                    };
                    symbols.push(symbol);
                }
            }
        }

        "enum_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = last_ident_of(bytes, name_node) {
                    let mut scoped = join_scope(module_stack);
                    scoped.push_str("::");
                    scoped.push_str(&name);

                    let symbol = SymbolDef {
                        name: interner.get_or_intern(&name),
                        scoped_name: interner.get_or_intern(&scoped),
                        kind: SymbolKind::Enum,
                        location: location_for(node, file),
                        signature: None,
                        visibility: extract_visibility(bytes, node),
                        attributes: extract_attributes(bytes, node),
                        doc_comment: extract_doc_comments(bytes, node),
                        parent: None,
                    };
                    symbols.push(symbol);
                }
            }
        }

        "trait_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = last_ident_of(bytes, name_node) {
                    let mut scoped = join_scope(module_stack);
                    scoped.push_str("::");
                    scoped.push_str(&name);

                    let symbol = SymbolDef {
                        name: interner.get_or_intern(&name),
                        scoped_name: interner.get_or_intern(&scoped),
                        kind: SymbolKind::Trait,
                        location: location_for(node, file),
                        signature: None,
                        visibility: extract_visibility(bytes, node),
                        attributes: extract_attributes(bytes, node),
                        doc_comment: extract_doc_comments(bytes, node),
                        parent: None,
                    };
                    symbols.push(symbol);
                }
            }
        }

        "const_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = last_ident_of(bytes, name_node) {
                    let mut scoped = join_scope(module_stack);

                    let parent = if let Some(ty) = current_impl_type(impl_type_stack) {
                        if !ty.is_empty() && ty != "_" {
                            scoped.push_str("::");
                            scoped.push_str(ty);
                            Some(interner.get_or_intern(ty))
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    scoped.push_str("::");
                    scoped.push_str(&name);

                    let symbol = SymbolDef {
                        name: interner.get_or_intern(&name),
                        scoped_name: interner.get_or_intern(&scoped),
                        kind: SymbolKind::Const,
                        location: location_for(node, file),
                        signature: None,
                        visibility: extract_visibility(bytes, node),
                        attributes: extract_attributes(bytes, node),
                        doc_comment: extract_doc_comments(bytes, node),
                        parent,
                    };
                    symbols.push(symbol);
                }
            }
        }

        "static_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = last_ident_of(bytes, name_node) {
                    let mut scoped = join_scope(module_stack);
                    scoped.push_str("::");
                    scoped.push_str(&name);

                    let symbol = SymbolDef {
                        name: interner.get_or_intern(&name),
                        scoped_name: interner.get_or_intern(&scoped),
                        kind: SymbolKind::Static,
                        location: location_for(node, file),
                        signature: None,
                        visibility: extract_visibility(bytes, node),
                        attributes: extract_attributes(bytes, node),
                        doc_comment: extract_doc_comments(bytes, node),
                        parent: None,
                    };
                    symbols.push(symbol);
                }
            }
        }

        "type_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = last_ident_of(bytes, name_node) {
                    let mut scoped = join_scope(module_stack);
                    scoped.push_str("::");
                    scoped.push_str(&name);

                    let symbol = SymbolDef {
                        name: interner.get_or_intern(&name),
                        scoped_name: interner.get_or_intern(&scoped),
                        kind: SymbolKind::TypeAlias,
                        location: location_for(node, file),
                        signature: None,
                        visibility: extract_visibility(bytes, node),
                        attributes: extract_attributes(bytes, node),
                        doc_comment: extract_doc_comments(bytes, node),
                        parent: None,
                    };
                    symbols.push(symbol);
                }
            }
        }

        _ => {}
    }

    // Recurse into children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            walk_rust_symbols(
                child,
                bytes,
                file,
                module_stack,
                impl_type_stack,
                interner,
                symbols,
            );
        }
    }

    // Exit scopes
    if entered_impl {
        impl_type_stack.pop();
    }
    if entered_mod {
        module_stack.pop();
    }
}

// ============================================================================
// Call Extraction Walker
// ============================================================================

fn walk_rust_calls(
    node: Node,
    bytes: &[u8],
    file: &Path,
    module_stack: &mut Vec<String>,
    impl_type_stack: &mut Vec<String>,
    fn_scope_stack: &mut Vec<String>,
    interner: &ThreadedRodeo,
    calls: &mut Vec<CallEdge>,
) {
    let kind = node.kind();

    // Enter inline module
    let mut entered_mod = false;
    if kind == "mod_item" {
        let has_body = node.child_by_field_name("body").is_some();
        if has_body {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Some(name) = last_ident_of(bytes, name_node) {
                    module_stack.push(name);
                    entered_mod = true;
                }
            }
        }
    }

    // Enter impl block
    let mut entered_impl = false;
    if kind == "impl_item" {
        if let Some(ty) = impl_type_ident(bytes, node) {
            impl_type_stack.push(ty);
            entered_impl = true;
        } else {
            impl_type_stack.push("_".to_string());
            entered_impl = true;
        }
    }

    // Enter function scope
    let mut entered_fn = false;
    if kind == "function_item" {
        if let Some(name_node) = node.child_by_field_name("name") {
            if let Some(fn_name) = last_ident_of(bytes, name_node) {
                let mut scoped = join_scope(module_stack);

                // Add impl type if inside impl block
                if let Some(ty) = current_impl_type(impl_type_stack) {
                    if !ty.is_empty() && ty != "_" {
                        scoped.push_str("::");
                        scoped.push_str(ty);
                    }
                }

                scoped.push_str("::");
                scoped.push_str(&fn_name);

                fn_scope_stack.push(scoped);
                entered_fn = true;
            }
        }
    }

    // Extract call expressions
    if kind == "call_expression" {
        if let Some(fun) = node.child_by_field_name("function") {
            if let Some(callee) = last_ident_of(bytes, fun) {
                let caller_scoped = fn_scope_stack
                    .last()
                    .cloned()
                    .unwrap_or_else(|| "crate::<toplevel>".to_string());

                // Check if it's a method call (has receiver)
                let is_method_call = fun.kind() == "field_expression";

                let call = CallEdge {
                    caller: interner.get_or_intern(&caller_scoped),
                    callee_name: callee,
                    location: location_for(node, file),
                    is_method_call,
                };
                calls.push(call);
            }
        }
    }

    // Recurse into children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            walk_rust_calls(
                child,
                bytes,
                file,
                module_stack,
                impl_type_stack,
                fn_scope_stack,
                interner,
                calls,
            );
        }
    }

    // Exit scopes
    if entered_fn {
        fn_scope_stack.pop();
    }
    if entered_impl {
        impl_type_stack.pop();
    }
    if entered_mod {
        module_stack.pop();
    }
}

// ============================================================================
// Import Extraction Walker
// ============================================================================

fn walk_rust_imports(
    node: Node,
    bytes: &[u8],
    file: &Path,
    imports: &mut Vec<ImportInfo>,
) {
    let kind = node.kind();

    if kind == "use_declaration" {
        // Extract the full use path
        if let Some(arg_node) = node.child_by_field_name("argument") {
            extract_use_tree(arg_node, bytes, file, "", imports);
        }
    }

    // Recurse into children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            walk_rust_imports(child, bytes, file, imports);
        }
    }
}

/// Extract import information from a use_tree node.
fn extract_use_tree(
    node: Node,
    bytes: &[u8],
    file: &Path,
    prefix: &str,
    imports: &mut Vec<ImportInfo>,
) {
    let kind = node.kind();

    match kind {
        "scoped_identifier" | "identifier" => {
            // Simple use: use foo::bar;
            if let Ok(text) = std::str::from_utf8(&bytes[node.start_byte()..node.end_byte()]) {
                let full_path = if prefix.is_empty() {
                    text.to_string()
                } else {
                    format!("{}::{}", prefix, text)
                };

                let name = text.split("::").last().unwrap_or(text).to_string();

                imports.push(ImportInfo {
                    path: full_path.clone(),
                    name,
                    is_glob: false,
                    location: location_for(node, file),
                });
            }
        }

        "use_as_clause" => {
            // Use with alias: use foo as bar;
            if let Some(path_node) = node.child_by_field_name("path") {
                if let Some(alias_node) = node.child_by_field_name("alias") {
                    if let Ok(path_text) = std::str::from_utf8(&bytes[path_node.start_byte()..path_node.end_byte()]) {
                        if let Ok(alias_text) = std::str::from_utf8(&bytes[alias_node.start_byte()..alias_node.end_byte()]) {
                            let full_path = if prefix.is_empty() {
                                path_text.to_string()
                            } else {
                                format!("{}::{}", prefix, path_text)
                            };

                            imports.push(ImportInfo {
                                path: full_path,
                                name: alias_text.to_string(),
                                is_glob: false,
                                location: location_for(node, file),
                            });
                        }
                    }
                }
            }
        }

        "use_wildcard" => {
            // Glob import: use foo::*; or use super::*;
            // The use_wildcard node can contain the path directly (e.g., super::*)
            // or be nested in a scoped_use_list

            let mut path_found = false;

            // Case 1: use_wildcard contains the path directly (e.g., "super::*")
            // Extract path by getting everything except :: and *
            let full_text = std::str::from_utf8(&bytes[node.start_byte()..node.end_byte()])
                .unwrap_or("");
            if full_text.ends_with("::*") {
                let path_part = &full_text[..full_text.len() - 3]; // Remove "::*"
                let full_path = if prefix.is_empty() {
                    path_part.to_string()
                } else {
                    format!("{}::{}", prefix, path_part)
                };

                imports.push(ImportInfo {
                    path: full_path,
                    name: "*".to_string(),
                    is_glob: true,
                    location: location_for(node, file),
                });
                path_found = true;
            }

            // Case 2: Try parent relationships for nested cases
            if !path_found {
                if let Some(parent) = node.parent() {
                    // use foo::{*} - nested in use_list
                    if parent.kind() == "use_list" {
                        if let Some(grandparent) = parent.parent() {
                            if grandparent.kind() == "scoped_use_list" {
                                if let Some(path_node) = grandparent.child_by_field_name("path") {
                                    if let Ok(path_text) = std::str::from_utf8(&bytes[path_node.start_byte()..path_node.end_byte()]) {
                                        let full_path = if prefix.is_empty() {
                                            path_text.to_string()
                                        } else {
                                            format!("{}::{}", prefix, path_text)
                                        };

                                        imports.push(ImportInfo {
                                            path: full_path,
                                            name: "*".to_string(),
                                            is_glob: true,
                                            location: location_for(node, file),
                                        });
                                        path_found = true;
                                    }
                                }
                            }
                        }
                    }
                    // use super::*; - direct scoped_use_list
                    else if parent.kind() == "scoped_use_list" {
                        if let Some(path_node) = parent.child_by_field_name("path") {
                            if let Ok(path_text) = std::str::from_utf8(&bytes[path_node.start_byte()..path_node.end_byte()]) {
                                let full_path = if prefix.is_empty() {
                                    path_text.to_string()
                                } else {
                                    format!("{}::{}", prefix, path_text)
                                };

                                imports.push(ImportInfo {
                                    path: full_path,
                                    name: "*".to_string(),
                                    is_glob: true,
                                    location: location_for(node, file),
                                });
                                path_found = true;
                            }
                        }
                    }
                }
            }

            // Fallback: if no path found, use prefix
            if !path_found && !prefix.is_empty() {
                imports.push(ImportInfo {
                    path: prefix.to_string(),
                    name: "*".to_string(),
                    is_glob: true,
                    location: location_for(node, file),
                });
            }
        }

        "scoped_use_list" => {
            // Nested use: use foo::{bar, baz};
            if let Some(path_node) = node.child_by_field_name("path") {
                if let Ok(path_text) = std::str::from_utf8(&bytes[path_node.start_byte()..path_node.end_byte()]) {
                    let new_prefix = if prefix.is_empty() {
                        path_text.to_string()
                    } else {
                        format!("{}::{}", prefix, path_text)
                    };

                    if let Some(list_node) = node.child_by_field_name("list") {
                        for i in 0..list_node.child_count() {
                            if let Some(child) = list_node.child(i) {
                                extract_use_tree(child, bytes, file, &new_prefix, imports);
                            }
                        }
                    }
                }
            }
        }

        "use_list" => {
            // Recurse into use list items
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    extract_use_tree(child, bytes, file, prefix, imports);
                }
            }
        }

        _ => {
            // Recurse into unknown nodes
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    extract_use_tree(child, bytes, file, prefix, imports);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lasso::ThreadedRodeo;
    use std::path::Path;
    use tree_sitter::Parser;

    #[test]
    fn test_extract_symbols() {
        let source = r#"
use std::collections::HashMap;

/// A test struct
pub struct MyStruct {
    value: i32,
}

impl MyStruct {
    pub fn new(value: i32) -> Self {
        Self { value }
    }

    fn internal(&self) -> i32 {
        self.value
    }
}

#[test]
fn test_something() {
    assert_eq!(1, 1);
}

pub fn public_function(x: i32) -> i32 {
    x + 1
}
"#;

        let rust_parser = RustParser::new();
        let mut parser = Parser::new();
        parser.set_language(&rust_parser.language()).unwrap();

        let tree = parser.parse(source, None).unwrap();
        let interner = ThreadedRodeo::default();

        let symbols = rust_parser
            .extract_symbols(&tree, source, Path::new("test.rs"), &interner)
            .unwrap();

        // Check that we found the expected symbols
        assert!(symbols.iter().any(|s| {
            let name = interner.resolve(&s.name);
            name == "MyStruct" && s.kind == SymbolKind::Struct
        }));

        assert!(symbols.iter().any(|s| {
            let name = interner.resolve(&s.name);
            name == "new" && s.kind == SymbolKind::Method
        }));

        assert!(symbols.iter().any(|s| {
            let name = interner.resolve(&s.name);
            name == "public_function" && s.kind == SymbolKind::Function
        }));

        // Check test function detection
        let test_fn = symbols.iter().find(|s| {
            let name = interner.resolve(&s.name);
            name == "test_something"
        });
        assert!(test_fn.is_some());
        let test_fn = test_fn.unwrap();
        assert!(test_fn.attributes.iter().any(|a| a.contains("test")));
    }

    #[test]
    fn test_extract_calls() {
        let source = r#"
fn caller() {
    callee();
    another_fn(1, 2);
}

fn callee() {}

fn another_fn(a: i32, b: i32) {}
"#;

        let rust_parser = RustParser::new();
        let mut parser = Parser::new();
        parser.set_language(&rust_parser.language()).unwrap();

        let tree = parser.parse(source, None).unwrap();
        let interner = ThreadedRodeo::default();

        let calls = rust_parser
            .extract_calls(&tree, source, Path::new("test.rs"), &interner)
            .unwrap();

        // Should find calls to callee and another_fn
        assert!(calls.iter().any(|c| c.callee_name == "callee"));
        assert!(calls.iter().any(|c| c.callee_name == "another_fn"));
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn test_extract_imports() {
        let source = r#"
use std::collections::HashMap;
use std::io::{Read, Write};
use super::*;
use crate::types::SymbolDef as Symbol;
"#;

        let rust_parser = RustParser::new();
        let mut parser = Parser::new();
        parser.set_language(&rust_parser.language()).unwrap();

        let tree = parser.parse(source, None).unwrap();

        let imports = rust_parser
            .extract_imports(&tree, source, Path::new("test.rs"))
            .unwrap();

        // Should find all imports
        assert!(imports.iter().any(|i| i.path.contains("HashMap")));
        assert!(imports.iter().any(|i| i.name == "Read"));
        assert!(imports.iter().any(|i| i.name == "Write"));
        assert!(imports.iter().any(|i| i.is_glob));
        assert!(imports.iter().any(|i| i.name == "Symbol"));
    }

    #[test]
    fn test_scoped_names() {
        let source = r#"
pub mod my_module {
    pub struct Foo;

    impl Foo {
        pub fn bar(&self) {}
    }
}
"#;

        let rust_parser = RustParser::new();
        let mut parser = Parser::new();
        parser.set_language(&rust_parser.language()).unwrap();

        let tree = parser.parse(source, None).unwrap();
        let interner = ThreadedRodeo::default();

        let symbols = rust_parser
            .extract_symbols(&tree, source, Path::new("test.rs"), &interner)
            .unwrap();

        // Check scoped names
        let bar_method = symbols.iter().find(|s| {
            let name = interner.resolve(&s.name);
            name == "bar"
        });
        assert!(bar_method.is_some());
        let scoped = interner.resolve(&bar_method.unwrap().scoped_name);
        assert_eq!(scoped, "crate::my_module::Foo::bar");
    }
}
