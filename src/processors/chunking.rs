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
    let mut ast_data = crate::processors::graph::extract_with_tree_sitter(content, language);

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
        let mut sorted_nodes = ast.defined_nodes.clone();
        sorted_nodes.sort_by_key(|n| n.start_byte);
        
        let mut last_end = 0;
        
        for node in &sorted_nodes {
            if node.start_byte > last_end && node.start_byte <= bytes.len() {
                // Extract loose code between last_end and node.start_byte
                let gap_bytes = &bytes[last_end..node.start_byte];
                if let Ok(snippet) = std::str::from_utf8(gap_bytes) {
                    if snippet.trim().len() > 0 {
                        let node_hash = compute_normalized_hash(snippet, language);
                        let mut loose_chunk = Chunk::new_stable(
                            source_id.to_string(),
                            file_path.to_string(),
                            snippet.to_string(),
                            ChunkType::Code {
                                language: language.to_string(),
                                semantic_type: CodeSemanticType::CodeBlock,
                            },
                            ChunkLevel::Semantic,
                            &format!("loose_code_{}", last_end),
                            &node_hash,
                        );
                        loose_chunk.metadata.line_range = Some((
                            content[..last_end].lines().count().max(1),
                            content[..node.start_byte].lines().count().max(1)
                        ));
                        chunks.push(loose_chunk);
                    }
                }
            }
            
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
                    if let Some(snippet_ast) = crate::processors::graph::extract_with_tree_sitter(snippet, language) {
                        node_chunk.metadata.ast_data = Some(snippet_ast);
                    }
                    
                    chunks.push(node_chunk);
                }
                if node.end_byte > last_end {
                    last_end = node.end_byte;
                }
            }
        }
        
        // Extract remaining loose code after the last node
        if last_end < bytes.len() {
            let gap_bytes = &bytes[last_end..];
            if let Ok(snippet) = std::str::from_utf8(gap_bytes) {
                if snippet.trim().len() > 0 {
                    let node_hash = compute_normalized_hash(snippet, language);
                    let mut loose_chunk = Chunk::new_stable(
                        source_id.to_string(),
                        file_path.to_string(),
                        snippet.to_string(),
                        ChunkType::Code {
                            language: language.to_string(),
                            semantic_type: CodeSemanticType::CodeBlock,
                        },
                        ChunkLevel::Semantic,
                        &format!("loose_code_{}", last_end),
                        &node_hash,
                    );
                    loose_chunk.metadata.line_range = Some((
                        content[..last_end].lines().count().max(1),
                        content.lines().count().max(1)
                    ));
                    chunks.push(loose_chunk);
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
