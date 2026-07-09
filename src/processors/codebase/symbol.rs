//! Cross-file symbol resolution for unified-processor graph.
//!
//! Replaces python `symbol_resolution.py` by building an in-memory index
//! of functions, classes, and types, then resolving references deterministically
//! across chunks.

use crate::core::chunking::Chunk;
use crate::graph::models::{ChunkRelationship, ChunkRelationType, RelationshipEvidence};
use fnv::FnvHashMap;
use uuid::Uuid;

/// A cross-file symbol index used for deterministic cross-chunk resolution.
pub struct SymbolIndex {
    /// Maps a function name -> chunk UUID where it was defined
    pub function_definitions: FnvHashMap<String, Vec<Uuid>>,
    
    /// Maps a class/struct/interface/type name -> chunk UUID where it was defined
    pub class_and_type_definitions: FnvHashMap<String, Vec<Uuid>>,

    /// Maps a file basename or module name -> chunk UUIDs (for resolving imports)
    pub file_paths: FnvHashMap<String, Vec<Uuid>>,
}

impl SymbolIndex {
    /// Build an index from a collection of chunks.
    pub fn build(chunks: &[Chunk]) -> Self {
        let mut function_definitions: FnvHashMap<String, Vec<Uuid>> = FnvHashMap::default();
        let mut class_and_type_definitions: FnvHashMap<String, Vec<Uuid>> = FnvHashMap::default();
        let mut file_paths: FnvHashMap<String, Vec<Uuid>> = FnvHashMap::default();

        for chunk in chunks {
            // Index file paths (both full and basename without extension)
            if let Some(path) = chunk.file_path.split('|').nth(1) {
                if let Some(file_name) = std::path::Path::new(path).file_name() {
                    let name = file_name.to_string_lossy().to_string();
                    file_paths.entry(name.clone()).or_default().push(chunk.id);
                    
                    // Also index without extension for module imports
                    if let Some(stem) = std::path::Path::new(path).file_stem() {
                        let stem_name = stem.to_string_lossy().to_string();
                        if stem_name != name {
                            file_paths.entry(stem_name).or_default().push(chunk.id);
                        }
                    }
                }
            }

            if let Some(ast_data) = &chunk.metadata.ast_data {
                for fn_name in &ast_data.function_names {
                    function_definitions.entry(fn_name.clone()).or_default().push(chunk.id);
                }
                for class_name in &ast_data.class_names {
                    class_and_type_definitions.entry(class_name.clone()).or_default().push(chunk.id);
                }
            }
        }

        Self {
            function_definitions,
            class_and_type_definitions,
            file_paths,
        }
    }

    /// Resolve function calls, instantiations, and type references into cross-chunk relationships.
    pub fn resolve_cross_file_references(&self, chunks: &[Chunk]) -> Vec<ChunkRelationship> {
        let mut new_edges = Vec::new();

        for chunk in chunks {
            if let Some(ast_data) = &chunk.metadata.ast_data {
                // 1. Resolve function calls
                for fn_call in &ast_data.function_calls {
                    if let Some(target_uuids) = self.function_definitions.get(fn_call) {
                        // Resolve unambiguously
                        if target_uuids.len() == 1 {
                            let target_uuid = target_uuids[0];
                            if chunk.id != target_uuid {
                                let edge = ChunkRelationship::new(
                                    chunk.id,
                                    target_uuid,
                                    ChunkRelationType::FunctionCalls,
                                    0.8, // INFERRED confidence
                                ).with_evidence(vec![
                                    RelationshipEvidence {
                                        evidence_type: "inferred_function_call".to_string(),
                                        location: "cross_file".to_string(),
                                        snippet: Some(fn_call.clone()),
                                    }
                                ]);
                                new_edges.push(edge);
                            }
                        }
                    }
                }

                // 2. Resolve instantiations
                for inst in &ast_data.instantiations {
                    if let Some(target_uuids) = self.class_and_type_definitions.get(inst) {
                        if target_uuids.len() == 1 {
                            let target_uuid = target_uuids[0];
                            if chunk.id != target_uuid {
                                let edge = ChunkRelationship::new(
                                    chunk.id,
                                    target_uuid,
                                    ChunkRelationType::Instantiates,
                                    0.8,
                                ).with_evidence(vec![
                                    RelationshipEvidence {
                                        evidence_type: "inferred_instantiation".to_string(),
                                        location: "cross_file".to_string(),
                                        snippet: Some(inst.clone()),
                                    }
                                ]);
                                new_edges.push(edge);
                            }
                        }
                    }
                }

                // 3. Resolve type references (from parameters, return types, fields)
                for type_ref in &ast_data.type_references {
                    if let Some(target_uuids) = self.class_and_type_definitions.get(&type_ref.type_name) {
                        if target_uuids.len() == 1 {
                            let target_uuid = target_uuids[0];
                            if chunk.id != target_uuid {
                                let edge = ChunkRelationship::new(
                                    chunk.id,
                                    target_uuid,
                                    ChunkRelationType::TypeReference,
                                    0.7,
                                ).with_evidence(vec![
                                    RelationshipEvidence {
                                        evidence_type: "inferred_type_ref".to_string(),
                                        location: type_ref.context.clone(),
                                        snippet: Some(type_ref.type_name.clone()),
                                    }
                                ]);
                                new_edges.push(edge);
                            }
                        }
                    }
                }
                // 4. Resolve file imports
                for import_path in &ast_data.import_paths {
                    // Extract basename or module name
                    let module_name = std::path::Path::new(import_path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| import_path.clone());

                    if let Some(target_uuids) = self.file_paths.get(&module_name) {
                        for target_uuid in target_uuids {
                            if chunk.id != *target_uuid {
                                let edge = ChunkRelationship::new(
                                    chunk.id,
                                    *target_uuid,
                                    ChunkRelationType::FileImports,
                                    0.9,
                                ).with_evidence(vec![
                                    RelationshipEvidence {
                                        evidence_type: "inferred_import".to_string(),
                                        location: "cross_file".to_string(),
                                        snippet: Some(import_path.clone()),
                                    }
                                ]);
                                new_edges.push(edge);
                            }
                        }
                    }
                }

                // 5. Resolve ClassInherits
                for parent_class in &ast_data.parent_classes {
                    if let Some(target_uuids) = self.class_and_type_definitions.get(parent_class) {
                        if target_uuids.len() == 1 {
                            let target_uuid = target_uuids[0];
                            if chunk.id != target_uuid {
                                let edge = ChunkRelationship::new(
                                    chunk.id,
                                    target_uuid,
                                    ChunkRelationType::ClassInherits,
                                    0.9,
                                ).with_evidence(vec![
                                    RelationshipEvidence {
                                        evidence_type: "inferred_inheritance".to_string(),
                                        location: "cross_file".to_string(),
                                        snippet: Some(parent_class.clone()),
                                    }
                                ]);
                                new_edges.push(edge);
                            }
                        }
                    }
                }

                // 6. Resolve Implements
                for (implementing_type, trait_name) in &ast_data.trait_implementations {
                    if let Some(target_uuids) = self.class_and_type_definitions.get(trait_name) {
                        if target_uuids.len() == 1 {
                            let target_uuid = target_uuids[0];
                            if chunk.id != target_uuid {
                                let edge = ChunkRelationship::new(
                                    chunk.id,
                                    target_uuid,
                                    ChunkRelationType::Implements,
                                    0.9,
                                ).with_evidence(vec![
                                    RelationshipEvidence {
                                        evidence_type: "inferred_implements".to_string(),
                                        location: "cross_file".to_string(),
                                        snippet: Some(format!("{} impl {}", implementing_type, trait_name)),
                                    }
                                ]);
                                new_edges.push(edge);
                            }
                        }
                    }
                }

                // 7. Resolve Decorates
                for decorator in &ast_data.decorators {
                    if let Some(target_uuids) = self.function_definitions.get(decorator).or_else(|| self.class_and_type_definitions.get(decorator)) {
                        if target_uuids.len() == 1 {
                            let target_uuid = target_uuids[0];
                            if chunk.id != target_uuid {
                                let edge = ChunkRelationship::new(
                                    chunk.id,
                                    target_uuid,
                                    ChunkRelationType::Decorates,
                                    0.8,
                                ).with_evidence(vec![
                                    RelationshipEvidence {
                                        evidence_type: "inferred_decorator".to_string(),
                                        location: "cross_file".to_string(),
                                        snippet: Some(decorator.clone()),
                                    }
                                ]);
                                new_edges.push(edge);
                            }
                        }
                    }
                }

                // 8. Resolve FunctionReferencesVariable (Map variables to functions with the same name as a fallback for constants/globals)
                for var_ref in &ast_data.variable_references {
                    if let Some(target_uuids) = self.function_definitions.get(var_ref) {
                        if target_uuids.len() == 1 {
                            let target_uuid = target_uuids[0];
                            if chunk.id != target_uuid {
                                let edge = ChunkRelationship::new(
                                    chunk.id,
                                    target_uuid,
                                    ChunkRelationType::FunctionReferencesVariable,
                                    0.6,
                                ).with_evidence(vec![
                                    RelationshipEvidence {
                                        evidence_type: "inferred_variable_ref".to_string(),
                                        location: "cross_file".to_string(),
                                        snippet: Some(var_ref.clone()),
                                    }
                                ]);
                                new_edges.push(edge);
                            }
                        }
                    }
                }
            }
        }

        new_edges
    }
}
