//! Cross-file symbol resolution for unified-processor graph.
//!
//! Replaces python `symbol_resolution.py` by building an in-memory index
//! of functions, classes, and types, then resolving references deterministically
//! across chunks.

use crate::core::chunking::Chunk;
use crate::graph::models::{ChunkRelationship, ChunkRelationType, RelationshipEvidence};
use crate::infra::storage::FalkordbStorage;
use fnv::FnvHashMap;
use std::collections::HashSet;
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

    /// Resolve function calls, instantiations, and type references by querying FalkorDB for targets missing from the current file.
    pub async fn resolve_cross_file_references_db(
        &self,
        chunks: &[Chunk],
        source_id: &str,
        user_graph: &FalkordbStorage,
    ) -> Vec<ChunkRelationship> {
        // First get the intra-file edges
        let mut new_edges = self.resolve_cross_file_references(chunks);

        let mut missing_functions = HashSet::new();
        let mut missing_classes = HashSet::new();
        let mut missing_imports = HashSet::new();

        // Collect unresolved cross-file targets
        for chunk in chunks {
            if let Some(ast_data) = &chunk.metadata.ast_data {
                for fn_call in &ast_data.function_calls {
                    if !self.function_definitions.contains_key(fn_call) {
                        missing_functions.insert((chunk.id, fn_call.clone()));
                    }
                }
                for decorator in &ast_data.decorators {
                    if !self.function_definitions.contains_key(decorator) && !self.class_and_type_definitions.contains_key(decorator) {
                        missing_functions.insert((chunk.id, decorator.clone()));
                    }
                }
                for inst in &ast_data.instantiations {
                    if !self.class_and_type_definitions.contains_key(inst) {
                        missing_classes.insert((chunk.id, inst.clone()));
                    }
                }
                for type_ref in &ast_data.type_references {
                    if !self.class_and_type_definitions.contains_key(&type_ref.type_name) {
                        missing_classes.insert((chunk.id, type_ref.type_name.clone()));
                    }
                }
                for class_inherits in &ast_data.parent_classes {
                    if !self.class_and_type_definitions.contains_key(class_inherits) {
                        missing_classes.insert((chunk.id, class_inherits.clone()));
                    }
                }
                for imp in &ast_data.import_paths {
                    let basename = imp.split('/').last().unwrap_or(imp).split('.').next().unwrap_or(imp);
                    if !self.file_paths.contains_key(basename) {
                        missing_imports.insert((chunk.id, basename.to_string(), imp.clone()));
                    }
                }
            }
        }

        // Helper to run query and parse single ID returns
        async fn resolve_batch(query: &str, user_graph: &FalkordbStorage) -> std::collections::HashMap<String, String> {
            let mut resolved = std::collections::HashMap::new();
            if let Ok(res) = user_graph.execute_query(query).await {
                let parsed = crate::infra::storage::parse_graphdb_response(res, &["key", "id"]);
                for row in parsed {
                    if let (Some(key), Some(id)) = (row.get("key").and_then(|v| v.as_str()), row.get("id").and_then(|v| v.as_str())) {
                        resolved.insert(key.to_string(), id.to_string());
                    }
                }
            }
            resolved
        }

        // 1. Resolve missing functions
        if !missing_functions.is_empty() {
            let func_names: Vec<String> = missing_functions.iter().map(|(_, name)| format!("'{}'", name.replace('\'', ""))).collect();
            let query = format!(
                "WITH [{}] AS names UNWIND names AS name MATCH (c:Vector_Chunk {{source_id: '{}'}}) WHERE c.chunk_key = 'Function_' + name RETURN name AS key, c.id AS id",
                func_names.join(", "), source_id
            );
            let resolved_funcs = resolve_batch(&query, user_graph).await;
            for (chunk_id, fn_call) in &missing_functions {
                if let Some(target_id) = resolved_funcs.get(fn_call) {
                    if let Ok(target_uuid) = Uuid::parse_str(target_id) {
                        new_edges.push(ChunkRelationship::new(
                            *chunk_id, target_uuid, ChunkRelationType::FunctionCalls, 0.8
                        ).with_evidence(vec![RelationshipEvidence {
                            evidence_type: "inferred_function_call".to_string(),
                            location: "falkordb_cross_file".to_string(),
                            snippet: Some(fn_call.clone()),
                        }]));
                    }
                }
            }
        }

        // 2. Resolve missing classes/types
        if !missing_classes.is_empty() {
            let class_names: Vec<String> = missing_classes.iter().map(|(_, name)| format!("'{}'", name.replace('\'', ""))).collect();
            let query = format!(
                "WITH [{}] AS names UNWIND names AS name MATCH (c:Vector_Chunk {{source_id: '{}'}}) WHERE c.chunk_key = 'Class_' + name RETURN name AS key, c.id AS id",
                class_names.join(", "), source_id
            );
            let resolved_classes = resolve_batch(&query, user_graph).await;
            for (chunk_id, class_name) in &missing_classes {
                if let Some(target_id) = resolved_classes.get(class_name) {
                    if let Ok(target_uuid) = Uuid::parse_str(target_id) {
                        new_edges.push(ChunkRelationship::new(
                            *chunk_id, target_uuid, ChunkRelationType::TypeReference, 0.7
                        ).with_evidence(vec![RelationshipEvidence {
                            evidence_type: "inferred_type_ref".to_string(),
                            location: "falkordb_cross_file".to_string(),
                            snippet: Some(class_name.clone()),
                        }]));
                    }
                }
            }
        }

        // 3. Resolve missing imports
        if !missing_imports.is_empty() {
            let import_names: Vec<String> = missing_imports.iter().map(|(_, name, _)| format!("'{}'", name.replace('\'', ""))).collect();
            let query = format!(
                "WITH [{}] AS names UNWIND names AS name MATCH (c:Vector_Chunk {{source_id: '{}', chunk_key: 'file_scope'}}) WHERE c.file_path CONTAINS name RETURN name AS key, c.id AS id",
                import_names.join(", "), source_id
            );
            let resolved_imports = resolve_batch(&query, user_graph).await;
            for (chunk_id, basename, full_import) in &missing_imports {
                if let Some(target_id) = resolved_imports.get(basename) {
                    if let Ok(target_uuid) = Uuid::parse_str(target_id) {
                        new_edges.push(ChunkRelationship::new(
                            *chunk_id, target_uuid, ChunkRelationType::FileImports, 0.9
                        ).with_evidence(vec![RelationshipEvidence {
                            evidence_type: "inferred_import".to_string(),
                            location: "falkordb_cross_file".to_string(),
                            snippet: Some(full_import.clone()),
                        }]));
                    }
                }
            }
        }

        new_edges
    }
}
