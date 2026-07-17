//! Code metrics calculation

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeMetrics {
    pub lines_of_code: usize,
    pub lines_of_comments: usize,
    pub cyclomatic_complexity: u32,
    pub cognitive_complexity: u32,
    pub maintainability_index: f32,
}


impl Default for CodeMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeMetrics {
    pub fn new() -> Self {
        Self {
            lines_of_code: 0,
            lines_of_comments: 0,
            cyclomatic_complexity: 0,
            cognitive_complexity: 0,
            maintainability_index: 0.0,
        }
    }

    pub fn calculate_quality_score(&self) -> f32 {
        let complexity_score = if self.cyclomatic_complexity <= 10 {
            100.0
        } else if self.cyclomatic_complexity <= 20 {
            80.0
        } else if self.cyclomatic_complexity <= 50 {
            60.0
        } else {
            40.0
        };

        let comment_ratio = if self.lines_of_code > 0 {
            (self.lines_of_comments as f32 / self.lines_of_code as f32) * 100.0
        } else {
            0.0
        };

        let comment_score = if comment_ratio >= 20.0 {
            100.0
        } else if comment_ratio >= 10.0 {
            80.0
        } else if comment_ratio >= 5.0 {
            60.0
        } else {
            40.0
        };

        (complexity_score + comment_score + self.maintainability_index) / 3.0
    }

    pub fn get_grade(&self) -> String {
        let score = self.calculate_quality_score();
        
        if score >= 90.0 {
            "A".to_string()
        } else if score >= 80.0 {
            "B".to_string()
        } else if score >= 70.0 {
            "C".to_string()
        } else if score >= 60.0 {
            "D".to_string()
        } else {
            "F".to_string()
        }
    }
}
