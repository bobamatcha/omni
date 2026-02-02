# Acceptance Criteria

The following are the minimum guarantees for omni as an agent-grade index + query tool:

- Query uses BM25 ranking.
- Query returns byte offsets plus 1-based line/column numbers.
- Default excludes: target/, node_modules/, .git/, dist/, build/, out/, coverage/, vendor/, .venv/, .next/, plus common lockfiles.
- Incremental indexing: unchanged files are not reparsed between runs.
