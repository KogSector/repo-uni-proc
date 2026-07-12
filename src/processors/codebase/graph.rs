//! Code-specific structural relationship extractor.
//!
//! Extracts relationships that are specific to `ChunkType::Code` chunks:
//! - Function call graphs (A calls B)
//! - Class inheritance (A inherits B)
//! - Trait/interface implementations (A implements B)
//! - Import dependencies (file A imports file B)
//! - Class-method containment (class A contains method B)
//! - Type references (function uses type from another chunk)
//! - Containment (file contains function/class)
//! - Inferred cross-file call resolution
//!
//! Non-code chunks are silently skipped.

use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::core::chunking::{Chunk, ChunkType, CodeSemanticType};
use crate::graph::models::{
    ASTData, ChunkRelationship, ChunkRelationshipMetadata, ChunkRelationType,
    RelationshipEvidence, TypeRef,
};
use crate::graph::extractors::SourceRelationshipExtractor;




/// Extracts structural relationships from code chunks.
pub struct CodeExtractor {}

impl CodeExtractor {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for CodeExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceRelationshipExtractor for CodeExtractor {
    fn source_type(&self) -> &'static str {
        "code"
    }

    fn extract(&self, chunks: &[Chunk]) -> Vec<ChunkRelationship> {
        let code_chunks: Vec<&Chunk> = chunks
            .iter()
            .filter(|c| matches!(c.chunk_type, ChunkType::Code { .. }))
            .collect();

        if code_chunks.is_empty() {
            return Vec::new();
        }

        // Build AST indices
        let mut function_to_chunk: HashMap<String, Uuid> = HashMap::new();
        let mut class_to_chunk: HashMap<String, Uuid> = HashMap::new();
        let mut file_to_chunk: HashMap<String, Uuid> = HashMap::new();

        // Extract AST signals for each code chunk
        let signals: Vec<(Uuid, ASTData, &ChunkType, &str)> = code_chunks
            .iter()
            .map(|chunk| {
                let signals = extract_structural_signals(chunk);

                // Index function names
                for fn_name in &signals.function_names {
                    function_to_chunk.insert(fn_name.clone(), chunk.id);
                }
                // Index class names
                for cls_name in &signals.class_names {
                    class_to_chunk.insert(cls_name.clone(), chunk.id);
                }
                // Index file-level chunks
                if let ChunkType::Code {
                    semantic_type: CodeSemanticType::File,
                    ..
                } = &chunk.chunk_type
                {
                    file_to_chunk.insert(chunk.file_path.clone(), chunk.id);
                }

                (chunk.id, signals, &chunk.chunk_type, chunk.file_path.as_str())
            })
            .collect();

        let mut relationships = Vec::new();
        // Track emitted edges to avoid duplicates
        let mut seen_edges: HashSet<(Uuid, Uuid, &'static str)> = HashSet::new();

        for (chunk_id, ast, chunk_type, file_path) in &signals {
            // ── Function calls (EXTRACTED — resolved within batch) ──────
            for called_fn in &ast.function_calls {
                if let Some(&target_id) = function_to_chunk.get(called_fn) {
                    if target_id != *chunk_id && seen_edges.insert((*chunk_id, target_id, "calls")) {
                        relationships.push(
                            ChunkRelationship::new(
                                *chunk_id,
                                target_id,
                                ChunkRelationType::FunctionCalls,
                                0.85,
                            )
                            .with_evidence(vec![RelationshipEvidence {
                                evidence_type: "function_call".to_string(),
                                location: file_path.to_string(),
                                snippet: Some(format!("calls {}()", called_fn)),
                            }])
                            .with_metadata(ChunkRelationshipMetadata {
                                extraction_method: "ast_analysis".to_string(),
                                source_chunk_type: "function".to_string(),
                                target_chunk_type: "function".to_string(),
                                ..Default::default()
                            }),
                        );
                    }
                }
            }

            // ── Class inheritance ───────────────────────────────────────
            for parent_class in &ast.parent_classes {
                if let Some(&target_id) = class_to_chunk.get(parent_class) {
                    if target_id != *chunk_id && seen_edges.insert((*chunk_id, target_id, "inherits")) {
                        relationships.push(
                            ChunkRelationship::new(
                                *chunk_id,
                                target_id,
                                ChunkRelationType::ClassInherits,
                                0.95,
                            )
                            .with_evidence(vec![RelationshipEvidence {
                                evidence_type: "inheritance".to_string(),
                                location: file_path.to_string(),
                                snippet: Some(format!("inherits {}", parent_class)),
                            }])
                            .with_metadata(ChunkRelationshipMetadata {
                                extraction_method: "ast_analysis".to_string(),
                                source_chunk_type: "class".to_string(),
                                target_chunk_type: "class".to_string(),
                                ..Default::default()
                            }),
                        );
                    }
                }
            }

            // ── Trait/interface implementations ─────────────────────────
            for (impl_type, trait_name) in &ast.trait_implementations {
                if let Some(&target_id) = class_to_chunk.get(trait_name) {
                    // The source is the implementing type's chunk
                    let source_id = class_to_chunk.get(impl_type).copied().unwrap_or(*chunk_id);
                    if target_id != source_id && seen_edges.insert((source_id, target_id, "implements")) {
                        relationships.push(
                            ChunkRelationship::new(
                                source_id,
                                target_id,
                                ChunkRelationType::Implements,
                                0.95,
                            )
                            .with_evidence(vec![RelationshipEvidence {
                                evidence_type: "trait_impl".to_string(),
                                location: file_path.to_string(),
                                snippet: Some(format!("{} implements {}", impl_type, trait_name)),
                            }])
                            .with_metadata(ChunkRelationshipMetadata {
                                extraction_method: "ast_analysis".to_string(),
                                source_chunk_type: "class".to_string(),
                                target_chunk_type: "trait".to_string(),
                                ..Default::default()
                            }),
                        );
                    }
                }
            }

            // ── Import dependencies ─────────────────────────────────────
            for import_path in &ast.import_paths {
                if let Some(target_id) = resolve_import(&file_to_chunk, import_path) {
                    if target_id != *chunk_id && seen_edges.insert((*chunk_id, target_id, "imports")) {
                        relationships.push(
                            ChunkRelationship::new(
                                *chunk_id,
                                target_id,
                                ChunkRelationType::FileImports,
                                0.90,
                            )
                            .with_evidence(vec![RelationshipEvidence {
                                evidence_type: "import_statement".to_string(),
                                location: file_path.to_string(),
                                snippet: Some(format!("imports {}", import_path)),
                            }])
                            .with_metadata(ChunkRelationshipMetadata {
                                extraction_method: "ast_analysis".to_string(),
                                source_chunk_type: "file".to_string(),
                                target_chunk_type: "file".to_string(),
                                ..Default::default()
                            }),
                        );
                    }
                }
            }

            // ── Class-contains-method ───────────────────────────────────
            if let ChunkType::Code {
                semantic_type: CodeSemanticType::Class,
                ..
            } = chunk_type
            {
                for (other_id, _other_ast, other_type, other_file) in &signals {
                    if other_id != chunk_id
                        && *other_file == *file_path
                        && matches!(
                            other_type,
                            ChunkType::Code {
                                semantic_type: CodeSemanticType::Function,
                                ..
                            }
                        )
                    {
                        if let Some(chunk) = code_chunks.iter().find(|c| c.id == *other_id) {
                            if chunk.metadata.parent_id == Some(*chunk_id)
                                && seen_edges.insert((*chunk_id, *other_id, "contains_method"))
                            {
                                relationships.push(
                                    ChunkRelationship::new(
                                        *chunk_id,
                                        *other_id,
                                        ChunkRelationType::ClassContainsMethod,
                                        0.95,
                                    )
                                    .with_evidence(vec![RelationshipEvidence {
                                        evidence_type: "class_method".to_string(),
                                        location: file_path.to_string(),
                                        snippet: None,
                                    }])
                                    .with_metadata(ChunkRelationshipMetadata {
                                        extraction_method: "structural".to_string(),
                                        source_chunk_type: "class".to_string(),
                                        target_chunk_type: "function".to_string(),
                                        ..Default::default()
                                    }),
                                );
                            }
                        }
                    }
                }
            }

            // ── Type references (Graphify-style) ────────────────────────
            // Resolve type names from parameters, returns, fields, generics
            // to class/struct chunks defined elsewhere in the codebase
            for type_ref in &ast.type_references {
                let target_id = class_to_chunk.get(&type_ref.type_name);
                if let Some(&tid) = target_id {
                    if tid != *chunk_id && seen_edges.insert((*chunk_id, tid, "type_ref")) {
                        relationships.push(
                            ChunkRelationship::new(
                                *chunk_id,
                                tid,
                                ChunkRelationType::TypeReference,
                                0.80,
                            )
                            .with_evidence(vec![RelationshipEvidence {
                                evidence_type: format!("type_ref_{}", type_ref.context),
                                location: file_path.to_string(),
                                snippet: Some(format!(
                                    "references type {} ({})", type_ref.type_name, type_ref.context
                                )),
                            }])
                            .with_metadata(ChunkRelationshipMetadata {
                                extraction_method: "ast_type_analysis".to_string(),
                                source_chunk_type: "function_or_struct".to_string(),
                                target_chunk_type: "class_or_struct".to_string(),
                                custom: {
                                    let mut map = std::collections::HashMap::new();
                                    map.insert("ref_context".to_string(), serde_json::json!(type_ref.context));
                                    map
                                },
                            }),
                        );
                    }
                }
            }

            // ── Instantiations ──────────────────────────────────────────
            for instantiated_class in &ast.instantiations {
                if let Some(&target_id) = class_to_chunk.get(instantiated_class) {
                    if target_id != *chunk_id && seen_edges.insert((*chunk_id, target_id, "instantiates")) {
                        relationships.push(
                            ChunkRelationship::new(
                                *chunk_id,
                                target_id,
                                ChunkRelationType::Instantiates,
                                0.80,
                            )
                            .with_evidence(vec![RelationshipEvidence {
                                evidence_type: "instantiation".to_string(),
                                location: file_path.to_string(),
                                snippet: Some(format!("instantiates {}", instantiated_class)),
                            }])
                            .with_metadata(ChunkRelationshipMetadata {
                                extraction_method: "ast_analysis".to_string(),
                                source_chunk_type: "function_or_class".to_string(),
                                target_chunk_type: "class".to_string(),
                                ..Default::default()
                            }),
                        );
                    }
                }
            }

            // ── Decorators ─────────────────────────────────────────────
            for decorator in &ast.decorators {
                let target_id = function_to_chunk.get(decorator)
                    .or_else(|| class_to_chunk.get(decorator));
                
                if let Some(&tid) = target_id {
                    if tid != *chunk_id && seen_edges.insert((*chunk_id, tid, "decorates")) {
                        relationships.push(
                            ChunkRelationship::new(
                                *chunk_id,
                                tid,
                                ChunkRelationType::Decorates,
                                0.90,
                            )
                            .with_evidence(vec![RelationshipEvidence {
                                evidence_type: "decorator".to_string(),
                                location: file_path.to_string(),
                                snippet: Some(format!("decorated by @{}", decorator)),
                            }])
                            .with_metadata(ChunkRelationshipMetadata {
                                extraction_method: "ast_analysis".to_string(),
                                source_chunk_type: "decorated_entity".to_string(),
                                target_chunk_type: "decorator_entity".to_string(),
                                ..Default::default()
                            }),
                        );
                    }
                }
            }
            // ── File defines entity ──────────────────────────────────────────
            if !matches!(chunk_type, ChunkType::Code { semantic_type: CodeSemanticType::File, .. }) {
                if let Some(&file_id) = file_to_chunk.get(*file_path) {
                    if file_id != *chunk_id && seen_edges.insert((*chunk_id, file_id, "defined_in")) {
                        relationships.push(
                            ChunkRelationship::new(
                                *chunk_id,
                                file_id,
                                ChunkRelationType::DefinedIn,
                                1.0,
                            )
                            .with_evidence(vec![RelationshipEvidence {
                                evidence_type: "definition".to_string(),
                                location: file_path.to_string(),
                                snippet: None,
                            }])
                            .with_metadata(ChunkRelationshipMetadata {
                                extraction_method: "structural".to_string(),
                                source_chunk_type: "entity".to_string(),
                                target_chunk_type: "file".to_string(),
                                ..Default::default()
                            }),
                        );
                    }
                }
            }
        }

        // ── Inferred cross-file call resolution (Graphify raw_calls style) ──
        // For function calls that didn't resolve within the same chunk,
        // try resolving across the entire batch using the global function index.
        for (chunk_id, ast, _chunk_type, file_path) in &signals {
            for called_fn in &ast.function_calls {
                // Skip if already resolved above (same chunk or EXTRACTED match)
                if function_to_chunk.get(called_fn).is_none_or(|&tid| tid == *chunk_id) {
                    continue; // Not found, or self-call — skip
                }
                // Already handled in the extracted pass above
            }

            // Also try resolving type names as function calls (constructors)
            // e.g., Python's MyClass() is both an instantiation and a call
            for type_ref in &ast.type_references {
                if let Some(&target_id) = function_to_chunk.get(&type_ref.type_name) {
                    if target_id != *chunk_id && seen_edges.insert((*chunk_id, target_id, "inferred_call")) {
                        relationships.push(
                            ChunkRelationship::new(
                                *chunk_id,
                                target_id,
                                ChunkRelationType::FunctionCalls,
                                0.65, // INFERRED confidence
                            )
                            .with_evidence(vec![RelationshipEvidence {
                                evidence_type: "inferred_call".to_string(),
                                location: file_path.to_string(),
                                snippet: Some(format!("inferred call to {}()", type_ref.type_name)),
                            }])
                            .with_metadata(ChunkRelationshipMetadata {
                                extraction_method: "inferred_cross_file".to_string(),
                                source_chunk_type: "function".to_string(),
                                target_chunk_type: "function".to_string(),
                                ..Default::default()
                            }),
                        );
                    }
                }
            }
        }


        tracing::info!(
            "[CodeExtractor] Extracted {} relationships: {} calls, {} type_refs, {} inherits, {} implements, {} imports, {} contains, {} instantiates, {} decorates",
            relationships.len(),
            relationships.iter().filter(|r| r.relationship_type == ChunkRelationType::FunctionCalls).count(),
            relationships.iter().filter(|r| r.relationship_type == ChunkRelationType::TypeReference).count(),
            relationships.iter().filter(|r| r.relationship_type == ChunkRelationType::ClassInherits).count(),
            relationships.iter().filter(|r| r.relationship_type == ChunkRelationType::Implements).count(),
            relationships.iter().filter(|r| r.relationship_type == ChunkRelationType::FileImports).count(),
            relationships.iter().filter(|r| r.relationship_type == ChunkRelationType::ClassContainsMethod).count(),
            relationships.iter().filter(|r| r.relationship_type == ChunkRelationType::Instantiates).count(),
            relationships.iter().filter(|r| r.relationship_type == ChunkRelationType::Decorates).count(),
        );

        relationships
    }
}


// ─── Signal extraction helpers ───────────────────────────────────────────────

/// Extract structural signals from a code chunk's content.
/// "Structural signals" is the source-agnostic term for what was previously
/// called "AST data" — it covers any parseable structural elements.
fn extract_structural_signals(chunk: &Chunk) -> ASTData {
    if let ChunkType::Code { language, .. } = &chunk.chunk_type {
        if let Some(ts_data) = extract_with_tree_sitter(&chunk.content, language) {
            tracing::debug!("Successfully used tree-sitter for {}", chunk.id);
            return ts_data;
        }
    }

    let mut data = ASTData::default();

    if let ChunkType::Code { language, semantic_type } = &chunk.chunk_type {
        data.language = Some(language.clone());

        match semantic_type {
            CodeSemanticType::Function => {
                if let Some(name) = extract_function_name(&chunk.content, language) {
                    data.function_names.push(name);
                }
                data.function_calls = extract_function_calls(&chunk.content);
                data.instantiations = extract_instantiations(&chunk.content, language);
                data.decorators = extract_decorators(&chunk.content, language);
                data.type_references = extract_type_references_regex(&chunk.content, language);
                data.trait_implementations = extract_trait_impls_regex(&chunk.content, language);
            }
            CodeSemanticType::Class => {
                if let Some(name) = extract_class_name(&chunk.content, language) {
                    data.class_names.push(name);
                }
                data.parent_classes = extract_parent_classes(&chunk.content, language);
                data.function_calls = extract_function_calls(&chunk.content);
                data.instantiations = extract_instantiations(&chunk.content, language);
                data.decorators = extract_decorators(&chunk.content, language);
                data.type_references = extract_type_references_regex(&chunk.content, language);
                data.trait_implementations = extract_trait_impls_regex(&chunk.content, language);
            }
            CodeSemanticType::File | CodeSemanticType::Module => {
                data.import_paths = extract_import_paths(&chunk.content, language);
                data.type_references = extract_type_references_regex(&chunk.content, language);
                data.trait_implementations = extract_trait_impls_regex(&chunk.content, language);
            }
            CodeSemanticType::Import => {
                data.import_paths = extract_import_paths(&chunk.content, language);
            }
            _ => {}
        }
    }

    data
}


/// Primitive/built-in types to exclude from type reference edges.
/// These are language-agnostic; language-specific builtins are filtered in each branch.
static PRIMITIVE_TYPES: &[&str] = &[
    "bool", "byte", "char", "i8", "i16", "i32", "i64", "i128", "isize",
    "u8", "u16", "u32", "u64", "u128", "usize", "f32", "f64",
    "str", "String", "string", "int", "float", "double", "void", "None",
    "true", "false", "null", "undefined", "object", "any", "never",
    "number", "boolean", "self", "Self", "super", "crate",
    "Vec", "Option", "Result", "Box", "Arc", "Rc", "HashMap", "HashSet",
    "List", "Dict", "Set", "Tuple", "Optional", "Union",
    "Array", "Map", "Promise", "Observable",
    "error", "rune", "complex64", "complex128", "uintptr", "comparable",
];

/// Helper: extract the unqualified type name from an AST node text.
/// E.g. "std::sync::Arc" → "Arc", "typing.Optional" → "Optional"
fn extract_type_name_from_text(text: &str, separator: &str) -> String {
    text.split(separator)
        .last()
        .unwrap_or(text)
        .trim()
        .trim_matches(&['<', '>', '(', ')', '[', ']', '&', '*', '?'] as &[char])
        .to_string()
}

/// Check if a type name is a primitive/builtin that shouldn't generate edges.
fn is_primitive_type(name: &str) -> bool {
    PRIMITIVE_TYPES.contains(&name) || name.is_empty()
}

/// Add a type reference if the name is non-primitive and not already seen.
fn add_type_ref(data: &mut ASTData, type_name: String, context: &str, seen: &mut HashSet<String>) {
    if !is_primitive_type(&type_name) && seen.insert(format!("{}:{}", type_name, context)) {
        data.type_references.push(TypeRef {
            type_name,
            context: context.to_string(),
        });
    }
}

pub fn extract_with_tree_sitter(content: &str, language: &str) -> Option<ASTData> {
    let mut parser = tree_sitter::Parser::new();
    
    let lang = match language {
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "c_sharp" => tree_sitter_c_sharp::LANGUAGE.into(),
        "c" => tree_sitter_c::LANGUAGE.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        "ruby" => tree_sitter_ruby::LANGUAGE.into(),
        "php" => tree_sitter_php::LANGUAGE_PHP.into(),
        "swift" => tree_sitter_swift::LANGUAGE.into(),
        "kotlin" => tree_sitter_kotlin::LANGUAGE.into(),
        "scala" => tree_sitter_scala::LANGUAGE.into(),
        "bash" => tree_sitter_bash::LANGUAGE.into(),
        _ => return None,
    };
    
    parser.set_language(&lang).ok()?;
    let tree = parser.parse(content, None)?;
    let root = tree.root_node();
    
    let mut data = ASTData::default();
    data.language = Some(language.to_string());
    
    let mut cursor = root.walk();
    let mut stack = vec![root];
    let bytes = content.as_bytes();
    // Track seen type refs per chunk to avoid duplicates
    let mut seen_type_refs: HashSet<String> = HashSet::new();
    
    while let Some(node) = stack.pop() {
        let kind = node.kind();
        
        // Generic extraction of defined nodes for semantic chunking
        if kind.contains("function") || kind.contains("method") || kind.contains("constructor") {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    data.defined_nodes.push(crate::graph::models::ASTNodeDef {
                        name: name.to_string(),
                        node_type: "Function".to_string(),
                        start_byte: node.start_byte(),
                        end_byte: node.end_byte(),
                        start_line: node.start_position().row,
                        end_line: node.end_position().row,
                    });
                }
            }
        } else if kind.contains("class") || kind.contains("struct") || kind.contains("trait") || kind.contains("enum") || kind.contains("interface") {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    data.defined_nodes.push(crate::graph::models::ASTNodeDef {
                        name: name.to_string(),
                        node_type: "Class".to_string(),
                        start_byte: node.start_byte(),
                        end_byte: node.end_byte(),
                        start_line: node.start_position().row,
                        end_line: node.end_position().row,
                    });
                }
            }
        }
        
        match language {
            "rust" => {
                match kind {
                    "function_item" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.function_names.push(name.to_string());
                            }
                        }
                        // Extract parameter type references
                        if let Some(params) = node.child_by_field_name("parameters") {
                            collect_type_refs_from_children(&params, bytes, "parameter_type", &mut data, &mut seen_type_refs, "::");
                        }
                        // Extract return type references
                        if let Some(ret) = node.child_by_field_name("return_type") {
                            collect_type_refs_from_node(&ret, bytes, "return_type", &mut data, &mut seen_type_refs, "::");
                        }
                    }
                    "struct_item" | "enum_item" | "trait_item" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.class_names.push(name.to_string());
                            }
                        }
                        // Struct field type references
                        if kind == "struct_item" {
                            for child_idx in 0..node.child_count() {
                                if let Some(child) = node.child(child_idx as u32) {
                                    if child.kind() == "field_declaration_list" {
                                        for field_idx in 0..child.child_count() {
                                            if let Some(field) = child.child(field_idx as u32) {
                                                if field.kind() == "field_declaration" {
                                                    if let Some(type_node) = field.child_by_field_name("type") {
                                                        collect_type_refs_from_node(&type_node, bytes, "field", &mut data, &mut seen_type_refs, "::");
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // Trait bounds → supertraits
                        if kind == "trait_item" {
                            for child_idx in 0..node.child_count() {
                                if let Some(child) = node.child(child_idx as u32) {
                                    if child.kind() == "trait_bounds" {
                                        collect_type_refs_from_node(&child, bytes, "generic_arg", &mut data, &mut seen_type_refs, "::");
                                    }
                                }
                            }
                        }
                    }
                    "impl_item" => {
                        // Extract impl Trait for Type → Implements relationship
                        let type_node = node.child_by_field_name("type");
                        let trait_node = node.child_by_field_name("trait");
                        if let (Some(type_n), Some(trait_n)) = (type_node, trait_node) {
                            if let (Ok(type_name), Ok(trait_name)) = (
                                type_n.utf8_text(bytes),
                                trait_n.utf8_text(bytes),
                            ) {
                                let type_name = extract_type_name_from_text(type_name, "::");
                                let trait_name = extract_type_name_from_text(trait_name, "::");
                                if !type_name.is_empty() && !trait_name.is_empty() {
                                    data.trait_implementations.push((type_name, trait_name));
                                }
                            }
                        }
                    }
                    "call_expression" => {
                        if let Some(func_node) = node.child_by_field_name("function") {
                            if let Ok(name) = func_node.utf8_text(bytes) {
                                let name = name.split("::").last().unwrap_or(name).to_string();
                                data.function_calls.push(name);
                            }
                        }
                    }
                    "use_declaration" => {
                        if let Ok(path) = node.utf8_text(bytes) {
                            let path = path.trim_start_matches("use ").trim_end_matches(';').trim().to_string();
                            data.import_paths.push(path);
                        }
                    }
                    _ => {}
                }
            }
            "python" => {
                match kind {
                    "function_definition" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.function_names.push(name.to_string());
                            }
                        }
                        // Extract parameter type annotations
                        if let Some(params) = node.child_by_field_name("parameters") {
                            for param_idx in 0..params.child_count() {
                                if let Some(param) = params.child(param_idx as u32) {
                                    // typed_parameter, typed_default_parameter
                                    if let Some(type_node) = param.child_by_field_name("type") {
                                        collect_type_refs_from_node(&type_node, bytes, "parameter_type", &mut data, &mut seen_type_refs, ".");
                                    }
                                }
                            }
                        }
                        // Extract return type annotation
                        if let Some(ret) = node.child_by_field_name("return_type") {
                            collect_type_refs_from_node(&ret, bytes, "return_type", &mut data, &mut seen_type_refs, ".");
                        }
                    }
                    "class_definition" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.class_names.push(name.to_string());
                            }
                        }
                        // Extract base classes / parent classes
                        if let Some(args) = node.child_by_field_name("superclasses") {
                            for child_idx in 0..args.child_count() {
                                if let Some(arg) = args.child(child_idx as u32) {
                                    if arg.is_named() {
                                        if let Ok(parent) = arg.utf8_text(bytes) {
                                            let parent = extract_type_name_from_text(parent, ".");
                                            if !parent.is_empty() && parent != "object" && parent != "ABC" {
                                                data.parent_classes.push(parent);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    "call" => {
                        if let Some(func_node) = node.child_by_field_name("function") {
                            if let Ok(name) = func_node.utf8_text(bytes) {
                                let name = name.split('.').next_back().unwrap_or(name).to_string();
                                data.function_calls.push(name);
                            }
                        }
                    }
                    "import_statement" | "import_from_statement" => {
                        if let Ok(path) = node.utf8_text(bytes) {
                            data.import_paths.push(path.to_string());
                        }
                    }
                    "decorator" => {
                        if let Ok(text) = node.utf8_text(bytes) {
                            let name = text.trim_start_matches('@').split('(').next().unwrap_or("").trim();
                            if !name.is_empty() {
                                data.decorators.push(name.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
            "javascript" | "typescript" => {
                match kind {
                    "function_declaration" | "method_definition" | "arrow_function" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.function_names.push(name.to_string());
                            }
                        }
                        // TS: Extract parameter type annotations
                        if language == "typescript" {
                            if let Some(params) = node.child_by_field_name("parameters") {
                                collect_type_refs_from_children(&params, bytes, "parameter_type", &mut data, &mut seen_type_refs, ".");
                            }
                            // Return type annotation
                            if let Some(ret) = node.child_by_field_name("return_type") {
                                collect_type_refs_from_node(&ret, bytes, "return_type", &mut data, &mut seen_type_refs, ".");
                            }
                        }
                    }
                    "class_declaration" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.class_names.push(name.to_string());
                            }
                        }
                        // JS/TS: class Foo extends Bar
                        for child_idx in 0..node.child_count() {
                            if let Some(child) = node.child(child_idx as u32) {
                                if child.kind() == "class_heritage" {
                                    for heritage_idx in 0..child.child_count() {
                                        if let Some(h) = child.child(heritage_idx as u32) {
                                            if h.kind() == "extends_clause" {
                                                if let Ok(text) = h.utf8_text(bytes) {
                                                    let parent = text.trim_start_matches("extends").trim();
                                                    if !parent.is_empty() {
                                                        data.parent_classes.push(parent.to_string());
                                                    }
                                                }
                                            }
                                            // TS: implements clause
                                            if h.kind() == "implements_clause" {
                                                if let Ok(text) = h.utf8_text(bytes) {
                                                    let iface = text.trim_start_matches("implements").trim();
                                                    let class_name = node.child_by_field_name("name")
                                                        .and_then(|n| n.utf8_text(bytes).ok())
                                                        .unwrap_or("");
                                                    for i in iface.split(',') {
                                                        let i = i.trim();
                                                        if !i.is_empty() && !class_name.is_empty() {
                                                            data.trait_implementations.push(
                                                                (class_name.to_string(), i.to_string())
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    "interface_declaration" => {
                        // TypeScript interfaces
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.class_names.push(name.to_string());
                            }
                        }
                    }
                    "type_alias_declaration" => {
                        // TypeScript type aliases
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.class_names.push(name.to_string());
                            }
                        }
                        // Extract type references from the type value
                        if let Some(value) = node.child_by_field_name("value") {
                            collect_type_refs_from_node(&value, bytes, "generic_arg", &mut data, &mut seen_type_refs, ".");
                        }
                    }
                    "call_expression" => {
                        if let Some(func_node) = node.child_by_field_name("function") {
                            if let Ok(name) = func_node.utf8_text(bytes) {
                                let name = name.split('.').next_back().unwrap_or(name).to_string();
                                data.function_calls.push(name);
                            }
                        }
                    }
                    "import_statement" => {
                        if let Some(source_node) = node.child_by_field_name("source") {
                            if let Ok(path) = source_node.utf8_text(bytes) {
                                let path = path.trim_matches('\'').trim_matches('"').to_string();
                                data.import_paths.push(path);
                            }
                        }
                    }
                    _ => {}
                }
            }
            "java" => {
                match kind {
                    "method_declaration" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.function_names.push(name.to_string());
                            }
                        }
                        // Extract parameter types
                        if let Some(params) = node.child_by_field_name("parameters") {
                            collect_type_refs_from_children(&params, bytes, "parameter_type", &mut data, &mut seen_type_refs, ".");
                        }
                        // Extract return type
                        if let Some(ret) = node.child_by_field_name("type") {
                            collect_type_refs_from_node(&ret, bytes, "return_type", &mut data, &mut seen_type_refs, ".");
                        }
                    }
                    "class_declaration" | "interface_declaration" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.class_names.push(name.to_string());
                            }
                        }
                        // Java: extends / implements
                        if let Some(superclass) = node.child_by_field_name("superclass") {
                            if let Ok(text) = superclass.utf8_text(bytes) {
                                let parent = extract_type_name_from_text(text, ".");
                                if !parent.is_empty() {
                                    data.parent_classes.push(parent);
                                }
                            }
                        }
                        if let Some(interfaces) = node.child_by_field_name("interfaces") {
                            if let Ok(text) = interfaces.utf8_text(bytes) {
                                let class_name = node.child_by_field_name("name")
                                    .and_then(|n| n.utf8_text(bytes).ok())
                                    .unwrap_or("");
                                for iface in text.split(',') {
                                    let iface = extract_type_name_from_text(iface.trim(), ".");
                                    if !iface.is_empty() && !class_name.is_empty() {
                                        data.trait_implementations.push(
                                            (class_name.to_string(), iface)
                                        );
                                    }
                                }
                            }
                        }
                    }
                    "field_declaration" => {
                        // Java class field types
                        if let Some(type_node) = node.child_by_field_name("type") {
                            collect_type_refs_from_node(&type_node, bytes, "field", &mut data, &mut seen_type_refs, ".");
                        }
                    }
                    "method_invocation" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.function_calls.push(name.to_string());
                            }
                        }
                    }
                    "import_declaration" => {
                        if let Ok(path) = node.utf8_text(bytes) {
                            let path = path.trim_start_matches("import ").trim_end_matches(';').trim().to_string();
                            data.import_paths.push(path);
                        }
                    }
                    "marker_annotation" | "annotation" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.decorators.push(name.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
            "go" => {
                match kind {
                    "function_declaration" | "method_declaration" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.function_names.push(name.to_string());
                            }
                        }
                        // Extract parameter types
                        if let Some(params) = node.child_by_field_name("parameters") {
                            collect_type_refs_from_children(&params, bytes, "parameter_type", &mut data, &mut seen_type_refs, ".");
                        }
                        // Extract return type
                        if let Some(ret) = node.child_by_field_name("result") {
                            collect_type_refs_from_node(&ret, bytes, "return_type", &mut data, &mut seen_type_refs, ".");
                        }
                    }
                    "type_spec" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.class_names.push(name.to_string());
                            }
                        }
                        // Go struct field types
                        if let Some(type_node) = node.child_by_field_name("type") {
                            if type_node.kind() == "struct_type" {
                                for field_idx in 0..type_node.child_count() {
                                    if let Some(field_list) = type_node.child(field_idx as u32) {
                                        if field_list.kind() == "field_declaration_list" {
                                            collect_type_refs_from_children(&field_list, bytes, "field", &mut data, &mut seen_type_refs, ".");
                                        }
                                    }
                                }
                            }
                            // Go interface embedding → implements
                            if type_node.kind() == "interface_type" {
                                let type_name = name_node_text(&node, bytes).unwrap_or_default();
                                for child_idx in 0..type_node.child_count() {
                                    if let Some(child) = type_node.child(child_idx as u32) {
                                        if child.kind() == "type_identifier" {
                                            if let Ok(iface) = child.utf8_text(bytes) {
                                                if !type_name.is_empty() && !iface.is_empty() {
                                                    data.trait_implementations.push(
                                                        (type_name.clone(), iface.to_string())
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    "call_expression" => {
                        if let Some(func_node) = node.child_by_field_name("function") {
                            if let Ok(name) = func_node.utf8_text(bytes) {
                                let name = name.split('.').next_back().unwrap_or(name).to_string();
                                data.function_calls.push(name);
                            }
                        }
                    }
                    "import_spec" => {
                        if let Some(path_node) = node.child_by_field_name("path") {
                            if let Ok(path) = path_node.utf8_text(bytes) {
                                let path = path.trim_matches('"').to_string();
                                data.import_paths.push(path);
                            }
                        }
                    }
                    _ => {}
                }
            }
            "c_sharp" => {
                match kind {
                    "method_declaration" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.function_names.push(name.to_string());
                            }
                        }
                        // Extract parameter types
                        if let Some(params) = node.child_by_field_name("parameters") {
                            collect_type_refs_from_children(&params, bytes, "parameter_type", &mut data, &mut seen_type_refs, ".");
                        }
                        // Extract return type
                        if let Some(ret) = node.child_by_field_name("returns") {
                            collect_type_refs_from_node(&ret, bytes, "return_type", &mut data, &mut seen_type_refs, ".");
                        }
                    }
                    "class_declaration" | "interface_declaration" | "record_declaration" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.class_names.push(name.to_string());
                            }
                        }
                        // C#: base_list → inherits / implements
                        for child_idx in 0..node.child_count() {
                            if let Some(child) = node.child(child_idx as u32) {
                                if child.kind() == "base_list" {
                                    let class_name = node.child_by_field_name("name")
                                        .and_then(|n| n.utf8_text(bytes).ok())
                                        .unwrap_or("");
                                    for base_idx in 0..child.child_count() {
                                        if let Some(base) = child.child(base_idx as u32) {
                                            if base.is_named() {
                                                if let Ok(text) = base.utf8_text(bytes) {
                                                    let base_name = extract_type_name_from_text(text, ".");
                                                    if !base_name.is_empty() && !class_name.is_empty() {
                                                        // Convention: IFoo → implements, else inherits
                                                        if base_name.starts_with('I') && base_name.len() > 1
                                                            && base_name.chars().nth(1).is_some_and(|c| c.is_uppercase())
                                                        {
                                                            data.trait_implementations.push(
                                                                (class_name.to_string(), base_name)
                                                            );
                                                        } else {
                                                            data.parent_classes.push(base_name);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    "field_declaration" | "property_declaration" => {
                        if let Some(type_node) = node.child_by_field_name("type") {
                            collect_type_refs_from_node(&type_node, bytes, "field", &mut data, &mut seen_type_refs, ".");
                        }
                    }
                    "invocation_expression" => {
                        if let Some(func_node) = node.child_by_field_name("function") {
                            if let Ok(name) = func_node.utf8_text(bytes) {
                                let name = name.split('.').next_back().unwrap_or(name).to_string();
                                data.function_calls.push(name);
                            }
                        }
                    }
                    "using_directive" => {
                        if let Ok(path) = node.utf8_text(bytes) {
                            let path = path.trim_start_matches("using ").trim_end_matches(';').trim().to_string();
                            data.import_paths.push(path);
                        }
                    }
                    "attribute" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.decorators.push(name.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
            "c" | "cpp" => {
                match kind {
                    "function_definition" => {
                        if let Some(decl_node) = node.child_by_field_name("declarator") {
                            if let Ok(name) = decl_node.utf8_text(bytes) {
                                let name = name.split('(').next().unwrap_or(name).trim().to_string();
                                let name = name.split("::").last().unwrap_or(&name).to_string();
                                data.function_names.push(name);
                            }
                        }
                        // Extract parameter types
                        if let Some(decl) = node.child_by_field_name("declarator") {
                            for child_idx in 0..decl.child_count() {
                                if let Some(child) = decl.child(child_idx as u32) {
                                    if child.kind() == "parameter_list" {
                                        collect_type_refs_from_children(&child, bytes, "parameter_type", &mut data, &mut seen_type_refs, "::");
                                    }
                                }
                            }
                        }
                        // Return type
                        if let Some(type_node) = node.child_by_field_name("type") {
                            collect_type_refs_from_node(&type_node, bytes, "return_type", &mut data, &mut seen_type_refs, "::");
                        }
                    }
                    "class_specifier" | "struct_specifier" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.class_names.push(name.to_string());
                            }
                        }
                        // C++: base_class_clause → inheritance
                        for child_idx in 0..node.child_count() {
                            if let Some(child) = node.child(child_idx as u32) {
                                if child.kind() == "base_class_clause" {
                                    if let Ok(text) = child.utf8_text(bytes) {
                                        // Parse ": public Base1, public Base2"
                                        for part in text.trim_start_matches(':').split(',') {
                                            let part = part.trim();
                                            let base = part
                                                .trim_start_matches("public ")
                                                .trim_start_matches("protected ")
                                                .trim_start_matches("private ")
                                                .trim_start_matches("virtual ")
                                                .trim();
                                            let base = extract_type_name_from_text(base, "::");
                                            if !base.is_empty() {
                                                data.parent_classes.push(base);
                                            }
                                        }
                                    }
                                }
                                // Struct fields
                                if child.kind() == "field_declaration_list" {
                                    for field_idx in 0..child.child_count() {
                                        if let Some(field) = child.child(field_idx as u32) {
                                            if field.kind() == "field_declaration" {
                                                if let Some(type_node) = field.child_by_field_name("type") {
                                                    collect_type_refs_from_node(&type_node, bytes, "field", &mut data, &mut seen_type_refs, "::");
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    "call_expression" => {
                        if let Some(func_node) = node.child_by_field_name("function") {
                            if let Ok(name) = func_node.utf8_text(bytes) {
                                let name = name.split("::").last().unwrap_or(name).to_string();
                                data.function_calls.push(name);
                            }
                        }
                    }
                    "preproc_include" => {
                        if let Some(path_node) = node.child_by_field_name("path") {
                            if let Ok(path) = path_node.utf8_text(bytes) {
                                let path = path.trim_matches('<').trim_matches('>').trim_matches('"').to_string();
                                data.import_paths.push(path);
                            }
                        }
                    }
                    _ => {}
                }
            }
            "ruby" => {
                match kind {
                    "method" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.function_names.push(name.to_string());
                            }
                        }
                    }
                    "class" | "module" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.class_names.push(name.to_string());
                            }
                        }
                        if let Some(superclass) = node.child_by_field_name("superclass") {
                            if let Ok(text) = superclass.utf8_text(bytes) {
                                let parent = extract_type_name_from_text(text, "::");
                                if !parent.is_empty() {
                                    data.parent_classes.push(parent);
                                }
                            }
                        }
                    }
                    "call" => {
                        if let Some(func_node) = node.child_by_field_name("method") {
                            if let Ok(name) = func_node.utf8_text(bytes) {
                                data.function_calls.push(name.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
            "php" => {
                match kind {
                    "method_declaration" | "function_definition" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.function_names.push(name.to_string());
                            }
                        }
                    }
                    "class_declaration" | "interface_declaration" | "trait_declaration" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.class_names.push(name.to_string());
                            }
                        }
                    }
                    "function_call_expression" | "member_call_expression" => {
                        if let Some(func_node) = node.child_by_field_name("function") {
                            if let Ok(name) = func_node.utf8_text(bytes) {
                                let name = name.split("->").last().unwrap_or(name).to_string();
                                let name = name.split("::").last().unwrap_or(&name).to_string();
                                data.function_calls.push(name);
                            }
                        } else if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.function_calls.push(name.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
            "swift" => {
                match kind {
                    "function_declaration" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.function_names.push(name.to_string());
                            }
                        }
                    }
                    "class_declaration" | "struct_declaration" | "protocol_declaration" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.class_names.push(name.to_string());
                            }
                        }
                    }
                    "call_expression" => {
                        for child_idx in 0..node.child_count() {
                            if let Some(child) = node.child(child_idx as u32) {
                                if child.kind() == "simple_identifier" || child.kind() == "member_access" {
                                    if let Ok(name) = child.utf8_text(bytes) {
                                        let name = name.split('.').next_back().unwrap_or(name).to_string();
                                        data.function_calls.push(name);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            "kotlin" => {
                match kind {
                    "function_declaration" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.function_names.push(name.to_string());
                            }
                        }
                    }
                    "class_declaration" | "object_declaration" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.class_names.push(name.to_string());
                            }
                        }
                    }
                    "call_expression" => {
                        for child_idx in 0..node.child_count() {
                            if let Some(child) = node.child(child_idx as u32) {
                                if child.kind() == "simple_identifier" || child.kind() == "navigation_expression" {
                                    if let Ok(name) = child.utf8_text(bytes) {
                                        let name = name.split('.').next_back().unwrap_or(name).to_string();
                                        data.function_calls.push(name);
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            "scala" => {
                match kind {
                    "function_definition" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.function_names.push(name.to_string());
                            }
                        }
                    }
                    "class_definition" | "object_definition" | "trait_definition" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.class_names.push(name.to_string());
                            }
                        }
                    }
                    "call_expression" => {
                        if let Some(func_node) = node.child_by_field_name("function") {
                            if let Ok(name) = func_node.utf8_text(bytes) {
                                let name = name.split('.').next_back().unwrap_or(name).to_string();
                                data.function_calls.push(name);
                            }
                        }
                    }
                    _ => {}
                }
            }
            "bash" => {
                match kind {
                    "function_definition" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.function_names.push(name.to_string());
                            }
                        }
                    }
                    "command" => {
                        if let Some(name_node) = node.child_by_field_name("name") {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                data.function_calls.push(name.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        
        for child in node.children(&mut cursor) {
            stack.push(child);
        }
    }
    
    Some(data)
}

/// Helper: get name text from a node's "name" field.
fn name_node_text<'a>(node: &tree_sitter::Node<'a>, bytes: &'a [u8]) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|n| n.utf8_text(bytes).ok())
        .map(|s| s.to_string())
}

/// Collect type references from named children of a node (e.g., parameter list).
fn collect_type_refs_from_children(
    node: &tree_sitter::Node,
    bytes: &[u8],
    context: &str,
    data: &mut ASTData,
    seen: &mut HashSet<String>,
    separator: &str,
) {
    for child_idx in 0..node.child_count() {
        if let Some(child) = node.child(child_idx as u32) {
            if child.is_named() {
                // Try to find a "type" field on each child (parameter)
                if let Some(type_node) = child.child_by_field_name("type") {
                    collect_type_refs_from_node(&type_node, bytes, context, data, seen, separator);
                }
            }
        }
    }
}

/// Collect type references from a single type node, recursing into generics.
fn collect_type_refs_from_node(
    node: &tree_sitter::Node,
    bytes: &[u8],
    context: &str,
    data: &mut ASTData,
    seen: &mut HashSet<String>,
    separator: &str,
) {
    if let Ok(text) = node.utf8_text(bytes) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        
        // Extract the primary type name (before generic args)
        let primary = text.split('<').next().unwrap_or(text)
            .split('[').next().unwrap_or(text);
        let type_name = extract_type_name_from_text(primary, separator);
        
        if !type_name.is_empty() {
            add_type_ref(data, type_name, context, seen);
        }
        
        // Extract generic type arguments (crude but effective)
        // e.g., Vec<MyStruct> → MyStruct as generic_arg
        if let Some(start) = text.find('<') {
            if let Some(end) = text.rfind('>') {
                let inner = &text[start + 1..end];
                for generic in inner.split(',') {
                    let generic = generic.trim();
                    let gen_name = extract_type_name_from_text(generic, separator);
                    if !gen_name.is_empty() {
                        add_type_ref(data, gen_name, "generic_arg", seen);
                    }
                }
            }
        }
    }
}

fn extract_function_name(content: &str, language: &str) -> Option<String> {
    let pattern = match language {
        "rust" => r"(?:pub\s+)?(?:async\s+)?fn\s+(\w+)",
        "python" => r"def\s+(\w+)",
        "javascript" | "typescript" => {
            r"(?:function\s+(\w+)|(?:const|let|var)\s+(\w+)\s*=\s*(?:async\s+)?(?:function|\())"
        }
        "go" => r"func\s+(\w+)",
        "java" => r"(?:public|private|protected)?\s*(?:static\s+)?[\w<>\[\]]+\s+(\w+)\s*\(",
        _ => return None,
    };

    let re = regex::Regex::new(pattern).ok()?;
    for line in content.lines().take(10) {
        if let Some(caps) = re.captures(line) {
            let name = caps.get(1).or(caps.get(2)).map(|m| m.as_str().to_string());
            if name.is_some() {
                return name;
            }
        }
    }
    None
}

fn extract_class_name(content: &str, language: &str) -> Option<String> {
    let pattern = match language {
        "rust" => r"(?:pub\s+)?(?:struct|enum|trait)\s+(\w+)",
        "python" => r"class\s+(\w+)",
        "javascript" | "typescript" => r"class\s+(\w+)",
        "java" => r"(?:public\s+)?class\s+(\w+)",
        "go" => r"type\s+(\w+)\s+struct",
        _ => return None,
    };

    let re = regex::Regex::new(pattern).ok()?;
    for line in content.lines().take(10) {
        if let Some(caps) = re.captures(line) {
            return caps.get(1).map(|m| m.as_str().to_string());
        }
    }
    None
}

fn extract_function_calls(content: &str) -> Vec<String> {
    static KEYWORDS: &[&str] = &[
        "if", "while", "for", "switch", "match", "return", "print",
        "fn", "def", "class", "struct", "enum", "impl", "trait",
        "let", "const", "var", "new", "self", "Self", "super",
        "pub", "async", "await", "yield", "import", "from",
    ];

    let re = match regex::Regex::new(r"([a-zA-Z_][a-zA-Z0-9_]*)\s*\(") {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let mut seen = std::collections::HashSet::new();
    let mut calls = Vec::new();

    for line in content.lines() {
        for caps in re.captures_iter(line) {
            if let Some(name) = caps.get(1) {
                let s = name.as_str();
                if !KEYWORDS.contains(&s) && seen.insert(s.to_string()) {
                    calls.push(s.to_string());
                }
            }
        }
    }
    calls
}

fn extract_import_paths(content: &str, language: &str) -> Vec<String> {
    let patterns: Vec<&str> = match language {
        "rust" => vec![r"use\s+([a-zA-Z_][a-zA-Z0-9_:]*)"],
        "python" => vec![r"from\s+(\S+)\s+import", r"import\s+(\S+)"],
        "javascript" | "typescript" => vec![r#"import\s+.*?from\s+['"]([^'"]+)['"]"#],
        "go" => vec![r#"import\s+(?:\(\s*)?["']([^"']+)["']"#],
        "java" => vec![r"import\s+([a-zA-Z_][a-zA-Z0-9_.]*)"],
        "html" => vec![
            r#"<script\s+[^>]*src\s*=\s*['"]([^'"]+)['"]"#,
            r#"<link\s+[^>]*href\s*=\s*['"]([^'"]+)['"]"#,
        ],
        "css" => vec![r#"@import\s+url\(['"]?([^'"()]+)['"]?\)"#, r#"@import\s+['"]([^'"]+)['"]"#],
        "c_sharp" => vec![r"using\s+([a-zA-Z_][a-zA-Z0-9_.]*)\s*;"],
        "c" | "cpp" => vec![r#"#include\s+"([^"]+)""#],
        "ruby" => vec![r#"require\s+['"]([^'"]+)['"]"#, r#"require_relative\s+['"]([^'"]+)['"]"#],
        "php" => vec![r#"(?:require|include)(?:_once)?\s+['"]([^'"]+)['"]"#, r"use\s+([a-zA-Z_\\][a-zA-Z0-9_\\]*)"],
        "swift" => vec![r"import\s+(\w+)"],
        "kotlin" => vec![r"import\s+([a-zA-Z_][a-zA-Z0-9_.]*)"],
        "scala" => vec![r"import\s+([a-zA-Z_][a-zA-Z0-9_.]*)"],
        "bash" => vec![r#"source\s+["']?([^"'\s]+)["']?"#, r#"\.\s+["']?([^"'\s]+)["']?"#],
        _ => return Vec::new(),
    };

    let mut imports = Vec::new();
    for pattern in patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            for line in content.lines() {
                if let Some(caps) = re.captures(line) {
                    if let Some(path) = caps.get(1) {
                        imports.push(path.as_str().to_string());
                    }
                }
            }
        }
    }
    imports
}

fn extract_parent_classes(content: &str, language: &str) -> Vec<String> {
    let pattern = match language {
        "rust" => r"impl\s+(\w+)\s+for",
        "python" => r"class\s+\w+\s*\(([^)]+)\)",
        "javascript" | "typescript" => r"class\s+\w+\s+extends\s+(\w+)",
        "java" => r"class\s+\w+\s+extends\s+(\w+)",
        _ => return Vec::new(),
    };

    let re = match regex::Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let mut parents = Vec::new();
    for line in content.lines() {
        if let Some(caps) = re.captures(line) {
            if let Some(parent) = caps.get(1) {
                if language == "python" {
                    for p in parent.as_str().split(',') {
                        let trimmed = p.trim();
                        if !trimmed.is_empty() && trimmed != "object" {
                            parents.push(trimmed.to_string());
                        }
                    }
                } else {
                    parents.push(parent.as_str().to_string());
                }
            }
        }
    }
    parents
}

fn resolve_import(file_to_chunk: &HashMap<String, Uuid>, import_path: &str) -> Option<Uuid> {
    if let Some(&id) = file_to_chunk.get(import_path) {
        return Some(id);
    }

    let variants = [
        import_path.replace("::", "/") + ".rs",
        import_path.replace("::", "/") + "/mod.rs",
        import_path.replace('.', "/") + ".py",
        import_path.replace('.', "/") + "/__init__.py",
        import_path.to_string() + ".js",
        import_path.to_string() + ".ts",
        import_path.to_string() + "/index.js",
        import_path.to_string() + "/index.ts",
    ];

    for variant in &variants {
        if let Some(&id) = file_to_chunk.get(variant.as_str()) {
            return Some(id);
        }
    }

    // Try finding exact path matches at the end of the file path
    // Remove leading ./ or ../ from relative imports to help matching
    let normalized_path = import_path.trim_start_matches("./").trim_start_matches("../");
    
    for (path, &id) in file_to_chunk {
        if path.ends_with(normalized_path) {
            return Some(id);
        }
    }

    // Fallback for languages where '.' means package directory separation (Java/Python)
    // Only apply this if the import path doesn't have a file extension
    if !import_path.contains(".js") && !import_path.contains(".ts") && !import_path.contains(".css") && !import_path.contains(".html") {
        let cleaned = import_path.replace("::", "/").replace('.', "/");
        for (path, &id) in file_to_chunk {
            if path.ends_with(&cleaned) || path.contains(&cleaned) {
                return Some(id);
            }
        }
    }

    None
}

fn extract_instantiations(content: &str, language: &str) -> Vec<String> {
    let pattern = match language {
        "rust" => r"(\w+)\s*\{", // Struct instantiation
        "python" | "javascript" | "typescript" | "java" => r"(?:new\s+)?(\w+)\s*\(",
        _ => return Vec::new(),
    };

    let re = match regex::Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let mut instantiations = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Skip keywords that might look like instantiations
    static KEYWORDS: &[&str] = &["if", "while", "for", "switch", "match", "return", "super", "self", "Self"];

    for line in content.lines() {
        for caps in re.captures_iter(line) {
            if let Some(name) = caps.get(1) {
                let s = name.as_str();
                if !KEYWORDS.contains(&s) && seen.insert(s.to_string()) {
                    instantiations.push(s.to_string());
                }
            }
        }
    }
    instantiations
}

fn extract_decorators(content: &str, language: &str) -> Vec<String> {
    let pattern = match language {
        "python" | "typescript" | "javascript" | "java" => r"@(\w+)",
        "rust" => r"#\[(\w+)", // Simple attribute
        _ => return Vec::new(),
    };

    let re = match regex::Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let mut decorators = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for line in content.lines() {
        for caps in re.captures_iter(line) {
            if let Some(name) = caps.get(1) {
                let s = name.as_str();
                if seen.insert(s.to_string()) {
                    decorators.push(s.to_string());
                }
            }
        }
    }
    decorators
}

/// Regex-based type reference extraction (fallback when tree-sitter unavailable).
fn extract_type_references_regex(content: &str, language: &str) -> Vec<TypeRef> {
    let mut refs = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Parameter type patterns per language
    let param_patterns: Vec<&str> = match language {
        "rust" => vec![
            r":\s*(?:&(?:mut\s+)?)?([A-Z][A-Za-z0-9_]+)",           // : Type or : &Type or : &mut Type
            r"->\s*(?:&(?:mut\s+)?)?([A-Z][A-Za-z0-9_]+)",          // -> ReturnType
            r"<([A-Z][A-Za-z0-9_]+)>",                               // Generic<Type>
        ],
        "python" => vec![
            r":\s*([A-Z][A-Za-z0-9_]+)",                             // : Type
            r"->\s*([A-Z][A-Za-z0-9_]+)",                            // -> ReturnType
        ],
        "typescript" | "javascript" => vec![
            r":\s*([A-Z][A-Za-z0-9_]+)",                             // : Type
        ],
        "java" | "c_sharp" => vec![
            r"\b([A-Z][A-Za-z0-9_]+)\s+\w+\s*[,;=)]",              // Type varName
            r"<([A-Z][A-Za-z0-9_]+)>",                               // Generic<Type>
        ],
        "go" => vec![
            r"\b\w+\s+([A-Z][A-Za-z0-9_]+)",                        // name Type (Go style)
            r"\*([A-Z][A-Za-z0-9_]+)",                               // *Type
        ],
        "c" | "cpp" => vec![
            r"\b([A-Z][A-Za-z0-9_]+)\s*[*&]?\s+\w+",               // Type* var or Type& var
            r"<([A-Z][A-Za-z0-9_]+)>",                               // template<Type>
        ],
        _ => return refs,
    };

    for pattern in param_patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            for caps in re.captures_iter(content) {
                if let Some(m) = caps.get(1) {
                    let name = m.as_str().to_string();
                    if !is_primitive_type(&name) && seen.insert(name.clone()) {
                        refs.push(TypeRef {
                            type_name: name,
                            context: "parameter_type".to_string(),
                        });
                    }
                }
            }
        }
    }

    refs
}

/// Regex-based trait/interface implementation extraction (fallback when tree-sitter unavailable).
fn extract_trait_impls_regex(content: &str, language: &str) -> Vec<(String, String)> {
    let mut impls = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let patterns: Vec<&str> = match language {
        "rust" => vec![r"impl\s+(\w+)\s+for\s+(\w+)"],
        "java" => vec![r"class\s+(\w+)\s+implements\s+([\w,\s]+)"],
        "c_sharp" => vec![r"class\s+(\w+)\s*:\s*([\w,\s]+)"],
        "typescript" => vec![r"class\s+(\w+)\s+implements\s+([\w,\s]+)"],
        "go" => vec![], // Go interfaces are implicit, regex can't detect them
        _ => return impls,
    };

    for pattern in patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            for caps in re.captures_iter(content) {
                match language {
                    "rust" => {
                        // impl Trait for Type → (Type, Trait)
                        if let (Some(trait_m), Some(type_m)) = (caps.get(1), caps.get(2)) {
                            let key = format!("{}:{}", type_m.as_str(), trait_m.as_str());
                            if seen.insert(key) {
                                impls.push((type_m.as_str().to_string(), trait_m.as_str().to_string()));
                            }
                        }
                    }
                    _ => {
                        // class Type implements/: Iface1, Iface2
                        if let (Some(type_m), Some(ifaces_m)) = (caps.get(1), caps.get(2)) {
                            let type_name = type_m.as_str();
                            for iface in ifaces_m.as_str().split(',') {
                                let iface = iface.trim();
                                if !iface.is_empty() {
                                    let key = format!("{}:{}", type_name, iface);
                                    if seen.insert(key) {
                                        impls.push((type_name.to_string(), iface.to_string()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    impls
}
