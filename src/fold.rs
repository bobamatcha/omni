//! Code folding and parsing utilities.
//!
//! Folds Rust source code to show only signatures, hiding implementation details.
//! Useful for context management and code summarization.

use anyhow::{Context, Result, anyhow};
use std::path::Path;
use tree_sitter::{Node, Parser};

/// Function signature information for single-file parsing.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct FunctionSignature {
    pub name: String,
    pub return_type: String,
    pub params: Vec<String>,
    pub is_public: bool,
}

/// Parse a single Rust file and return its function definitions.
/// Useful for tools that need to parse code without building a full index.
pub fn parse_single_file(
    source: &str,
    _file_path: &Path,
) -> Result<Vec<(String, FunctionSignature)>> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| anyhow!("parser.set_language: {e:?}"))?;

    let tree = parser
        .parse(source, None)
        .context("Failed to parse source")?;

    let mut results = Vec::new();
    collect_functions(tree.root_node(), source, &mut results)?;

    Ok(results)
}

/// Collect function signatures from AST nodes.
fn collect_functions(
    node: Node,
    source: &str,
    results: &mut Vec<(String, FunctionSignature)>,
) -> Result<()> {
    let kind = node.kind();

    match kind {
        "function_item" => {
            if let Some(sig) = extract_function_signature(node, source) {
                results.push((sig.name.clone(), sig));
            }
        }
        "impl_item" => {
            // Process methods inside impl
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "declaration_list" {
                    let mut inner_cursor = child.walk();
                    for inner_child in child.children(&mut inner_cursor) {
                        if inner_child.kind() == "function_item" {
                            if let Some(sig) = extract_function_signature(inner_child, source) {
                                results.push((sig.name.clone(), sig));
                            }
                        }
                    }
                }
            }
        }
        _ => {
            // Recurse into children
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_functions(child, source, results)?;
            }
        }
    }

    Ok(())
}

/// Extract function signature from a function_item node.
fn extract_function_signature(node: Node, source: &str) -> Option<FunctionSignature> {
    let mut name = String::new();
    let mut return_type = String::new();
    let mut params = Vec::new();
    let mut is_public = false;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "visibility_modifier" => {
                let text = &source[child.start_byte()..child.end_byte()];
                is_public = text.contains("pub");
            }
            "identifier" => {
                name = source[child.start_byte()..child.end_byte()].to_string();
            }
            "parameters" => {
                // Parse each parameter
                let mut param_cursor = child.walk();
                for param in child.children(&mut param_cursor) {
                    if param.kind() == "parameter" || param.kind() == "self_parameter" {
                        let param_text = &source[param.start_byte()..param.end_byte()];
                        params.push(param_text.to_string());
                    }
                }
            }
            "type_identifier" | "generic_type" | "reference_type" | "primitive_type"
            | "tuple_type" | "array_type" | "unit_type" | "pointer_type" | "function_type" => {
                // This is the return type after ->
                return_type = source[child.start_byte()..child.end_byte()].to_string();
            }
            _ => {}
        }
    }

    // Also check for return type in the text between ) and {
    if return_type.is_empty() {
        let fn_text = &source[node.start_byte()..node.end_byte()];
        if let Some(arrow_pos) = fn_text.find("->") {
            let after_arrow = &fn_text[arrow_pos + 2..];
            if let Some(brace_pos) = after_arrow.find('{') {
                return_type = after_arrow[..brace_pos].trim().to_string();
            } else if let Some(semi_pos) = after_arrow.find(';') {
                return_type = after_arrow[..semi_pos].trim().to_string();
            }
        }
    }

    if !name.is_empty() {
        Some(FunctionSignature {
            name,
            return_type,
            params,
            is_public,
        })
    } else {
        None
    }
}

/// Fold Rust source code to show only signatures.
///
/// Keeps:
/// - Use declarations
/// - Struct/enum/trait definitions
/// - Function signatures (bodies replaced with `{ /* ... */ }`)
/// - Impl blocks with folded methods
/// - Doc comments
///
/// # Example
///
/// ```ignore
/// // Input:
/// pub fn calculate(x: i32, y: i32) -> i32 {
///     let result = x + y;
///     result * 2
/// }
///
/// // Output:
/// pub fn calculate(x: i32, y: i32) -> i32 { /* ... */ }
/// ```
pub fn fold_to_signatures(source: &str, _file_path: &Path) -> Result<String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|e| anyhow!("parser.set_language: {e:?}"))?;

    let tree = parser
        .parse(source, None)
        .context("Failed to parse source")?;
    let root_node = tree.root_node();

    let mut output = String::new();
    fold_node(root_node, source, &mut output)?;

    Ok(output)
}

/// Recursively fold AST nodes, keeping signatures but eliding bodies.
fn fold_node(node: Node, source: &str, output: &mut String) -> Result<()> {
    let kind = node.kind();

    match kind {
        // Keep entire node as-is (imports, type aliases, etc.)
        "use_declaration" | "mod_item" | "type_item" | "extern_crate_declaration" => {
            let text = &source[node.start_byte()..node.end_byte()];
            output.push_str(text);
            output.push('\n');
        }

        // Struct/enum: keep definition, fold impl blocks
        "struct_item" | "enum_item" | "union_item" | "trait_item" => {
            let text = &source[node.start_byte()..node.end_byte()];
            output.push_str(text);
            output.push('\n');
        }

        // Constants/statics: keep signature, elide value
        "const_item" | "static_item" => {
            let text = &source[node.start_byte()..node.end_byte()];
            if let Some(eq_pos) = text.find('=') {
                output.push_str(&text[..eq_pos]);
                output.push_str("= /* ... */;\n");
            } else {
                output.push_str(text);
                output.push('\n');
            }
        }

        // Function: keep signature, replace body with placeholder
        "function_item" => {
            fold_function(node, source, output)?;
        }

        // Impl block: keep header, fold methods
        "impl_item" => {
            fold_impl(node, source, output)?;
        }

        // Source file or other container: recurse into children
        "source_file" | "declaration_list" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                fold_node(child, source, output)?;
            }
        }

        // Attributes: keep them (they precede the item they annotate)
        "attribute_item" | "inner_attribute_item" => {
            let text = &source[node.start_byte()..node.end_byte()];
            output.push_str(text);
            output.push('\n');
        }

        // Line/block comments at top level: keep doc comments
        "line_comment" | "block_comment" => {
            let text = &source[node.start_byte()..node.end_byte()];
            // Keep doc comments (/// or //!) but skip regular comments
            if text.starts_with("///") || text.starts_with("//!") || text.starts_with("/**") {
                output.push_str(text);
                output.push('\n');
            }
        }

        // Macro invocations at module level (like macro_rules!)
        "macro_invocation" | "macro_definition" => {
            let text = &source[node.start_byte()..node.end_byte()];
            // Keep short macros, summarize long ones
            if text.len() < 200 {
                output.push_str(text);
            } else {
                // Just keep the macro name
                if let Some(name_node) = node.child_by_field_name("macro") {
                    let name = &source[name_node.start_byte()..name_node.end_byte()];
                    output.push_str(&format!("{}! {{ /* ... */ }}", name));
                } else {
                    output.push_str("/* macro elided */");
                }
            }
            output.push('\n');
        }

        // Skip everything else (expressions, etc.)
        _ => {}
    }

    Ok(())
}

/// Fold a function item: keep signature, replace body with placeholder.
fn fold_function(node: Node, source: &str, output: &mut String) -> Result<()> {
    // Get attributes (preceding siblings)
    let mut attrs = Vec::new();
    let mut prev = node.prev_sibling();
    while let Some(sib) = prev {
        if sib.kind() == "attribute_item" {
            attrs.push(&source[sib.start_byte()..sib.end_byte()]);
            prev = sib.prev_sibling();
        } else {
            break;
        }
    }
    // Attributes were collected in reverse order
    for attr in attrs.iter().rev() {
        output.push_str(attr);
        output.push('\n');
    }

    // Build signature: everything up to the body
    let mut sig_end = node.end_byte();
    if let Some(body_node) = node.child_by_field_name("body") {
        sig_end = body_node.start_byte();
    }

    let signature = &source[node.start_byte()..sig_end];
    output.push_str(signature.trim_end());
    output.push_str(" { /* ... */ }\n");

    Ok(())
}

/// Fold an impl block: keep header, fold methods.
fn fold_impl(node: Node, source: &str, output: &mut String) -> Result<()> {
    // Find the impl header (everything before the body/declaration_list)
    let mut header_end = node.end_byte();
    let mut body_node = None;

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "declaration_list" {
            header_end = child.start_byte();
            body_node = Some(child);
            break;
        }
    }

    let header = &source[node.start_byte()..header_end];
    output.push_str(header.trim_end());
    output.push_str(" {\n");

    // Fold methods in the body
    if let Some(body) = body_node {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "function_item" => {
                    output.push_str("    ");
                    // Inline fold for method
                    let mut sig_end = child.end_byte();
                    if let Some(fn_body) = child.child_by_field_name("body") {
                        sig_end = fn_body.start_byte();
                    }
                    let sig = &source[child.start_byte()..sig_end];
                    output.push_str(sig.trim_end());
                    output.push_str(" { /* ... */ }\n");
                }
                "const_item" | "type_item" => {
                    output.push_str("    ");
                    let text = &source[child.start_byte()..child.end_byte()];
                    if let Some(eq_pos) = text.find('=') {
                        output.push_str(&text[..eq_pos]);
                        output.push_str("= /* ... */;\n");
                    } else {
                        output.push_str(text);
                        output.push('\n');
                    }
                }
                "attribute_item" => {
                    output.push_str("    ");
                    let text = &source[child.start_byte()..child.end_byte()];
                    output.push_str(text);
                    output.push('\n');
                }
                _ => {}
            }
        }
    }

    output.push_str("}\n");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_fold_function() {
        let source = r#"
pub fn calculate(x: i32, y: i32) -> i32 {
    let result = x + y;
    result * 2
}
"#;
        let folded = fold_to_signatures(source, Path::new("test.rs")).unwrap();
        assert!(folded.contains("pub fn calculate(x: i32, y: i32) -> i32"));
        assert!(folded.contains("{ /* ... */ }"));
        assert!(!folded.contains("let result"));
    }

    #[test]
    fn test_fold_struct_and_impl() {
        let source = r#"
pub struct Point {
    x: f64,
    y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }

    pub fn distance(&self, other: &Point) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}
"#;
        let folded = fold_to_signatures(source, Path::new("test.rs")).unwrap();
        assert!(folded.contains("pub struct Point"));
        assert!(folded.contains("impl Point"));
        assert!(folded.contains("pub fn new(x: f64, y: f64) -> Self"));
        assert!(folded.contains("pub fn distance(&self, other: &Point) -> f64"));
        assert!(!folded.contains("let dx"));
    }

    #[test]
    fn test_fold_keeps_use() {
        let source = r#"
use std::path::Path;
use std::collections::HashMap;

fn foo() {
    println!("Hello");
}
"#;
        let folded = fold_to_signatures(source, Path::new("test.rs")).unwrap();
        assert!(folded.contains("use std::path::Path;"));
        assert!(folded.contains("use std::collections::HashMap;"));
    }

    #[test]
    fn test_fold_keeps_doc_comments() {
        let source = r#"
/// This is a doc comment
pub fn documented() {
    // implementation
}
"#;
        let folded = fold_to_signatures(source, Path::new("test.rs")).unwrap();
        assert!(folded.contains("/// This is a doc comment"));
    }
}
