use crate::core::chunking::{Chunk, ChunkLevel, ChunkType, CodeSemanticType};
use sha2::{Digest, Sha256};

/// Normalize content by removing non-semantic whitespace.
pub fn normalize_content(content: &str, _language: &str) -> String {
    let mut normalized = String::with_capacity(content.len());
    let mut last_was_space = false;
    
    for c in content.chars() {
        if c.is_whitespace() {
            if !last_was_space {
                normalized.push(' ');
                last_was_space = true;
            }
        } else {
            normalized.push(c);
            last_was_space = false;
        }
    }
    
    normalized.trim().to_string()
}

/// Computes the SHA256 hash of the normalized content
pub fn compute_normalized_hash(content: &str, language: &str) -> String {
    let normalized = normalize_content(content, language);
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let hash_bytes = hasher.finalize();
    hash_bytes.iter().map(|b| format!("{:02x}", b)).collect()
}





pub fn extract_ast_chunks(
    source_id: &str,
    file_path: &str,
    content: &str,
    language: &str,
) -> Vec<Chunk> {
    let mut ast_data = crate::processors::codebase::graph::extract_with_tree_sitter(content, language);

    let chunk_hash = compute_normalized_hash(content, language);
    let mut chunks = Vec::new();
    
    // Create the root File chunk
    let mut file_chunk = Chunk::new_stable(
        source_id.to_string(),
        file_path.to_string(),
        content.to_string(),
        ChunkType::Code {
            language: language.to_string(),
            semantic_type: CodeSemanticType::File,
        },
        ChunkLevel::Semantic,
        "file_scope",
        &chunk_hash,
    );
    file_chunk.metadata.line_range = Some((1, content.lines().count()));
    file_chunk.metadata.ast_data = ast_data.clone(); // The file chunk gets the full AST
    chunks.push(file_chunk);

    // If we have granular defined nodes, create chunks for them
    if let Some(ast) = &mut ast_data {
        let bytes = content.as_bytes();
        for node in &ast.defined_nodes {
            if node.end_byte > node.start_byte && node.end_byte <= bytes.len() {
                if let Ok(snippet) = std::str::from_utf8(&bytes[node.start_byte..node.end_byte]) {
                    let sem_type = if node.node_type == "Class" {
                        CodeSemanticType::Class
                    } else {
                        CodeSemanticType::Function
                    };
                    
                    let node_hash = compute_normalized_hash(snippet, language);
                    let mut node_chunk = Chunk::new_stable(
                        source_id.to_string(),
                        file_path.to_string(),
                        snippet.to_string(),
                        ChunkType::Code {
                            language: language.to_string(),
                            semantic_type: sem_type,
                        },
                        ChunkLevel::Semantic,
                        &format!("{}_{}", node.node_type, node.name),
                        &node_hash,
                    );
                    
                    // The line numbers from tree-sitter are 0-indexed
                    node_chunk.metadata.line_range = Some((node.start_line + 1, node.end_line + 1));
                    
                    // Extract ASTData specifically for this chunk so SymbolIndex creates granular edges
                    if let Some(snippet_ast) = crate::processors::codebase::graph::extract_with_tree_sitter(snippet, language) {
                        node_chunk.metadata.ast_data = Some(snippet_ast);
                    }
                    
                    chunks.push(node_chunk);
                }
            }
        }
        
        // Remove granular data from File chunk to avoid duplicate edges
        ast.function_names.clear();
        ast.class_names.clear();
        ast.defined_nodes.clear();
    }

    chunks
}
