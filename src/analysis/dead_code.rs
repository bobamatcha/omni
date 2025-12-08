//! Dead code analysis via global reachability.
//!
//! This module performs whole-program analysis to identify potentially dead code
//! by building a reachability graph from entry points (main functions, tests, public APIs)
//! and marking all symbols that are transitively called from those entry points.

use crate::state::OciState;
use crate::types::{DeadCodeReport, InternedString, SymbolKind, Visibility};
use std::collections::{HashSet, VecDeque};

/// Analyzes code to detect potentially dead (unreachable) symbols.
pub struct DeadCodeAnalyzer;

impl DeadCodeAnalyzer {
    /// Creates a new dead code analyzer.
    pub fn new() -> Self {
        Self
    }

    /// Performs dead code analysis on the entire codebase.
    ///
    /// This works by:
    /// 1. Identifying entry points (main, tests, public symbols, trait impls)
    /// 2. Performing BFS from entry points following call edges
    /// 3. Marking all reachable symbols as live
    /// 4. Reporting unreachable symbols as potentially dead
    pub fn analyze(&self, state: &OciState) -> DeadCodeReport {
        // Step 1: Identify all entry points
        let entry_points = self.identify_entry_points(state);

        // Step 2: Perform reachability analysis from entry points
        let reachable = self.compute_reachable(state, &entry_points);

        // Step 3: Identify dead symbols (symbols not in reachable set)
        let dead_symbols = self.find_dead_symbols(state, &reachable);

        // Step 4: Identify potentially live symbols (conservative estimation)
        // These are symbols that might be used through dynamic dispatch, FFI, etc.
        let potentially_live = self.identify_potentially_live(state, &reachable);

        DeadCodeReport {
            dead_symbols,
            entry_points,
            potentially_live,
        }
    }

    /// Identifies entry points for reachability analysis.
    ///
    /// Entry points include:
    /// - Functions named "main"
    /// - Functions with #[test] attribute
    /// - Public symbols (pub/pub(crate))
    /// - Trait implementations
    fn identify_entry_points(&self, state: &OciState) -> Vec<InternedString> {
        let mut entry_points = Vec::new();

        // Iterate over all symbols
        for entry in state.symbols.iter() {
            let scoped_name = *entry.key();
            let symbol = entry.value();

            // Check if this is an entry point
            if self.is_entry_point(state, symbol) {
                entry_points.push(scoped_name);
            }
        }

        entry_points
    }

    /// Determines if a symbol is an entry point.
    fn is_entry_point(
        &self,
        state: &OciState,
        symbol: &crate::types::SymbolDef,
    ) -> bool {
        // 1. Functions named "main" are always entry points
        let name = state.resolve(symbol.name);
        if name == "main" && matches!(symbol.kind, SymbolKind::Function) {
            return true;
        }

        // 2. Functions with #[test] attribute
        if symbol.attributes.iter().any(|attr| attr.contains("test")) {
            return true;
        }

        // 3. Public symbols are entry points (can be called from outside)
        if matches!(symbol.visibility, Visibility::Public) {
            return true;
        }

        // 4. pub(crate) symbols are also entry points (reachable within crate)
        if matches!(symbol.visibility, Visibility::Crate) {
            return true;
        }

        // 5. Trait implementations are entry points (can be called through trait objects)
        if matches!(symbol.kind, SymbolKind::Impl) {
            return true;
        }

        // 6. Methods in trait impls are entry points
        if matches!(symbol.kind, SymbolKind::Method) {
            // Check if parent is an impl
            if let Some(parent) = symbol.parent {
                if let Some(parent_sym) = state.get_symbol(parent) {
                    if matches!(parent_sym.kind, SymbolKind::Impl) {
                        return true;
                    }
                }
            }
        }

        false
    }

    /// Computes the set of reachable symbols from the given entry points.
    ///
    /// Uses BFS to traverse the call graph.
    fn compute_reachable(
        &self,
        state: &OciState,
        entry_points: &[InternedString],
    ) -> HashSet<InternedString> {
        let mut reachable = HashSet::new();
        let mut queue = VecDeque::new();

        // Initialize with entry points
        for &entry in entry_points {
            reachable.insert(entry);
            queue.push_back(entry);
        }

        // BFS traversal
        while let Some(current) = queue.pop_front() {
            // Find all callees of the current symbol
            let callees = state.find_callees(current);

            for call_edge in callees {
                // Resolve callee name to scoped symbols
                let callee_symbols = state.find_by_name(&call_edge.callee_name);

                for callee_sym in callee_symbols {
                    let callee_scoped = callee_sym.scoped_name;

                    // If we haven't seen this symbol yet, mark it as reachable
                    if reachable.insert(callee_scoped) {
                        queue.push_back(callee_scoped);
                    }
                }
            }

            // Also mark parent symbols as reachable
            // (if a method is reachable, its struct/impl should be too)
            if let Some(symbol) = state.get_symbol(current) {
                if let Some(parent) = symbol.parent {
                    if reachable.insert(parent) {
                        queue.push_back(parent);
                    }
                }
            }
        }

        reachable
    }

    /// Finds symbols that are not reachable (potentially dead).
    fn find_dead_symbols(
        &self,
        state: &OciState,
        reachable: &HashSet<InternedString>,
    ) -> Vec<InternedString> {
        let mut dead_symbols = Vec::new();

        for entry in state.symbols.iter() {
            let scoped_name = *entry.key();
            let symbol = entry.value();

            // Skip if reachable
            if reachable.contains(&scoped_name) {
                continue;
            }

            // Skip certain symbol kinds that are not meaningful for dead code analysis
            if matches!(
                symbol.kind,
                SymbolKind::Module | SymbolKind::Field | SymbolKind::Variant
            ) {
                continue;
            }

            dead_symbols.push(scoped_name);
        }

        dead_symbols
    }

    /// Identifies symbols that are potentially live through non-standard mechanisms.
    ///
    /// These include:
    /// - Symbols with macro attributes (might be called by macro expansion)
    /// - Symbols with derive attributes (might be used by generated code)
    /// - Constants and statics (might be used through references)
    fn identify_potentially_live(
        &self,
        state: &OciState,
        reachable: &HashSet<InternedString>,
    ) -> Vec<InternedString> {
        let mut potentially_live = Vec::new();

        for entry in state.symbols.iter() {
            let scoped_name = *entry.key();
            let symbol = entry.value();

            // Skip if already reachable
            if reachable.contains(&scoped_name) {
                continue;
            }

            // Check for attributes that might indicate dynamic usage
            let has_special_attrs = symbol.attributes.iter().any(|attr| {
                attr.contains("derive")
                    || attr.contains("macro")
                    || attr.contains("no_mangle")
                    || attr.contains("export_name")
                    || attr.contains("link_name")
                    || attr.contains("used")
            });

            if has_special_attrs {
                potentially_live.push(scoped_name);
                continue;
            }

            // Constants and statics might be used through references
            if matches!(symbol.kind, SymbolKind::Const | SymbolKind::Static) {
                potentially_live.push(scoped_name);
                continue;
            }

            // Macros are often used in ways not tracked by call graph
            if matches!(symbol.kind, SymbolKind::Macro) {
                potentially_live.push(scoped_name);
            }
        }

        potentially_live
    }
}

impl Default for DeadCodeAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::create_state;
    use crate::types::{Location, Signature, SymbolDef};
    use std::path::PathBuf;

    #[test]
    fn test_entry_point_detection() {
        let state = create_state(PathBuf::from("/test"));
        let analyzer = DeadCodeAnalyzer::new();

        // Add a main function
        let main_name = state.intern("main");
        let main_scoped = state.intern("crate::main");
        state.add_symbol(SymbolDef {
            name: main_name,
            scoped_name: main_scoped,
            kind: SymbolKind::Function,
            location: Location::new(PathBuf::from("/test/main.rs"), 0, 10),
            signature: Some(Signature::default()),
            visibility: Visibility::Private,
            attributes: vec![],
            doc_comment: None,
            parent: None,
        });

        // Add a test function
        let test_name = state.intern("test_foo");
        let test_scoped = state.intern("crate::test_foo");
        state.add_symbol(SymbolDef {
            name: test_name,
            scoped_name: test_scoped,
            kind: SymbolKind::Function,
            location: Location::new(PathBuf::from("/test/main.rs"), 20, 30),
            signature: Some(Signature::default()),
            visibility: Visibility::Private,
            attributes: vec!["test".to_string()],
            doc_comment: None,
            parent: None,
        });

        // Add a public function
        let pub_name = state.intern("public_api");
        let pub_scoped = state.intern("crate::public_api");
        state.add_symbol(SymbolDef {
            name: pub_name,
            scoped_name: pub_scoped,
            kind: SymbolKind::Function,
            location: Location::new(PathBuf::from("/test/main.rs"), 40, 50),
            signature: Some(Signature::default()),
            visibility: Visibility::Public,
            attributes: vec![],
            doc_comment: None,
            parent: None,
        });

        // Add a private function (not an entry point)
        let priv_name = state.intern("internal");
        let priv_scoped = state.intern("crate::internal");
        state.add_symbol(SymbolDef {
            name: priv_name,
            scoped_name: priv_scoped,
            kind: SymbolKind::Function,
            location: Location::new(PathBuf::from("/test/main.rs"), 60, 70),
            signature: Some(Signature::default()),
            visibility: Visibility::Private,
            attributes: vec![],
            doc_comment: None,
            parent: None,
        });

        let entry_points = analyzer.identify_entry_points(&state);

        // Should have 3 entry points: main, test, and public
        assert_eq!(entry_points.len(), 3);
        assert!(entry_points.contains(&main_scoped));
        assert!(entry_points.contains(&test_scoped));
        assert!(entry_points.contains(&pub_scoped));
        assert!(!entry_points.contains(&priv_scoped));
    }

    #[test]
    fn test_reachability_analysis() {
        let state = create_state(PathBuf::from("/test"));
        let analyzer = DeadCodeAnalyzer::new();

        // Add main function
        let main_scoped = state.intern("crate::main");
        state.add_symbol(SymbolDef {
            name: state.intern("main"),
            scoped_name: main_scoped,
            kind: SymbolKind::Function,
            location: Location::new(PathBuf::from("/test/main.rs"), 0, 10),
            signature: Some(Signature::default()),
            visibility: Visibility::Private,
            attributes: vec![],
            doc_comment: None,
            parent: None,
        });

        // Add helper function (called by main)
        let helper_scoped = state.intern("crate::helper");
        state.add_symbol(SymbolDef {
            name: state.intern("helper"),
            scoped_name: helper_scoped,
            kind: SymbolKind::Function,
            location: Location::new(PathBuf::from("/test/main.rs"), 20, 30),
            signature: Some(Signature::default()),
            visibility: Visibility::Private,
            attributes: vec![],
            doc_comment: None,
            parent: None,
        });

        // Add dead function (not called)
        let dead_scoped = state.intern("crate::dead");
        state.add_symbol(SymbolDef {
            name: state.intern("dead"),
            scoped_name: dead_scoped,
            kind: SymbolKind::Function,
            location: Location::new(PathBuf::from("/test/main.rs"), 40, 50),
            signature: Some(Signature::default()),
            visibility: Visibility::Private,
            attributes: vec![],
            doc_comment: None,
            parent: None,
        });

        // Add call edge: main -> helper
        use crate::types::CallEdge;
        state.add_call_edge(CallEdge {
            caller: main_scoped,
            callee_name: "helper".to_string(),
            location: Location::new(PathBuf::from("/test/main.rs"), 5, 6),
            is_method_call: false,
        });

        let report = analyzer.analyze(&state);

        // main is entry point
        assert!(report.entry_points.contains(&main_scoped));

        // helper is reachable
        assert!(!report.dead_symbols.contains(&helper_scoped));

        // dead is not reachable
        assert!(report.dead_symbols.contains(&dead_scoped));
    }
}
