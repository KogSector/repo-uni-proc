use crate::core::Result;
use super::metrics::CodeMetrics;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeData {
    pub filename: String,
    pub language: String,
    pub functions: Vec<FunctionInfo>,
    pub classes: Vec<ClassInfo>,
    pub imports: Vec<String>,
    pub metrics: CodeMetrics,
    pub ast_summary: AstSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionInfo {
    pub name: String,
    pub line_number: usize,
    pub parameters: Vec<String>,
    pub return_type: Option<String>,
    pub complexity: u32,
    pub docstring: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassInfo {
    pub name: String,
    pub line_number: usize,
    pub methods: Vec<FunctionInfo>,
    pub properties: Vec<String>,
    pub inheritance: Vec<String>,
    pub docstring: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AstSummary {
    pub total_nodes: usize,
    pub max_depth: usize,
    pub node_types: HashMap<String, usize>,
    pub syntax_errors: Vec<SyntaxError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntaxError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

pub struct CodeAnalyzer {}

impl CodeAnalyzer {
    pub fn new() -> Result<Self> {
        Ok(Self {})
    }

    pub async fn analyze_code(&self, content: &str, filename: &str) -> Result<CodeData> {
        let language = self.detect_language(filename);
        let lines_of_code = content.lines().count();
        let mut functions = Vec::new();
        let mut classes = Vec::new();
        let mut total_complexity = 0;
        let mut max_complexity = 0;
        
        let (func_pattern, class_pattern) = match language.as_str() {
            "rust" => (r"(?:pub\s+)?(?:async\s+)?fn\s+(\w+)", r"(?:pub\s+)?(?:struct|enum|trait)\s+(\w+)"),
            "python" => (r"def\s+(\w+)", r"class\s+(\w+)"),
            "javascript" | "typescript" => (r"(?:function\s+(\w+)|(?:const|let|var)\s+(\w+)\s*=\s*(?:async\s+)?(?:function|\())", r"class\s+(\w+)"),
            "go" => (r"func\s+(\w+)", r"type\s+(\w+)\s+struct"),
            "java" => (r"(?:public|private|protected)?\s*(?:static\s+)?[\w<>\[\]]+\s+(\w+)\s*\(", r"(?:public\s+)?class\s+(\w+)"),
            _ => (r"(?:fn|def|function|func)\s+(\w+)", r"class\s+(\w+)"), // generic fallback
        };
        
        if let (Ok(f_re), Ok(c_re)) = (regex::Regex::new(func_pattern), regex::Regex::new(class_pattern)) {
            for (i, line) in content.lines().enumerate() {
                // Approximate cyclomatic complexity per line
                let complexity_keywords = ["if ", "while ", "for ", "match ", "switch ", "&&", "||", "? "];
                let mut line_complexity = 0;
                for kw in complexity_keywords {
                    if line.contains(kw) {
                        line_complexity += 1;
                    }
                }
                
                total_complexity += line_complexity;
                if line_complexity > max_complexity {
                    max_complexity = line_complexity;
                }
                
                if let Some(caps) = f_re.captures(line) {
                    let name = caps.get(1).or(caps.get(2)).map(|m| m.as_str().to_string()).unwrap_or_default();
                    if !name.is_empty() {
                        functions.push(FunctionInfo {
                            name,
                            line_number: i + 1,
                            parameters: vec![],
                            return_type: None,
                            complexity: 1 + line_complexity,
                            docstring: None,
                        });
                    }
                } else if let Some(caps) = c_re.captures(line) {
                    let name = caps.get(1).map(|m| m.as_str().to_string()).unwrap_or_default();
                    if !name.is_empty() {
                        classes.push(ClassInfo {
                            name,
                            line_number: i + 1,
                            methods: vec![],
                            properties: vec![],
                            inheritance: vec![],
                            docstring: None,
                        });
                    }
                }
            }
        }
        
        let mut metrics = CodeMetrics::new();
        metrics.lines_of_code = lines_of_code;
        metrics.cyclomatic_complexity = total_complexity;
        metrics.cognitive_complexity = max_complexity;
        
        let ast_summary = AstSummary {
            total_nodes: functions.len() + classes.len(),
            max_depth: 0,
            node_types: HashMap::new(),
            syntax_errors: vec![],
        };
        
        Ok(CodeData {
            filename: filename.to_string(),
            language,
            functions,
            classes,
            imports: vec![], // No longer extracted via legacy regex
            metrics,
            ast_summary,
        })
    }

    pub fn detect_language(&self, filename: &str) -> String {
        static EXT_TO_LANG: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
            [
                ("rs", "rust"),
                ("py", "python"),
                ("js", "javascript"),
                ("jsx", "javascript"),
                ("ts", "typescript"),
                ("tsx", "typescript"),
                ("go", "go"),
                ("java", "java"),
                ("c", "c"),
                ("cpp", "cpp"),
                ("cxx", "cpp"),
                ("cc", "cpp"),
                ("h", "c"),
                ("hpp", "cpp"),
                ("hxx", "cpp"),
                ("cs", "c_sharp"),
                ("rb", "ruby"),
                ("html", "html"),
                ("htm", "html"),
                ("css", "css"),
            ]
            .into_iter()
            .collect()
        });

        let extension = std::path::Path::new(filename)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_lowercase();

        EXT_TO_LANG
            .get(extension.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    }

    pub fn supported_languages(&self) -> Vec<String> {
        vec!["rust".to_string(), "python".to_string(), "javascript".to_string(), "typescript".to_string(), "go".to_string(), "java".to_string(), "c".to_string(), "cpp".to_string(), "c_sharp".to_string(), "ruby".to_string(), "html".to_string(), "css".to_string()]
    }
}

