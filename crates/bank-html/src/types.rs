use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QType {
    Single,
    Multiple,
    Judge,
    Fill,
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionItem {
    pub key: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Answer {
    Single(String),
    Multiple(Vec<String>),
    Judge(bool),
    Fill(Vec<String>),
    Code(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub qtype: QType,
    pub stem: String,
    pub stem_html: String,
    pub options: Vec<OptionItem>,
    pub answer: Answer,
    pub source: String,
    pub paper_title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BankDoc {
    pub title: String,
    pub questions: Vec<Question>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportFailure {
    pub index: usize,
    pub reason: String,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportReport {
    pub filename: String,
    pub title: String,
    pub success: Vec<Question>,
    pub failures: Vec<ImportFailure>,
}

impl Question {
    pub fn semantic_eq(&self, other: &Question) -> bool {
        self.qtype == other.qtype
            && normalize_text(&self.stem) == normalize_text(&other.stem)
            && option_texts(&self.options) == option_texts(&other.options)
            && answers_eq(&self.answer, &other.answer)
            && normalize_text(&self.source) == normalize_text(&other.source)
    }
}

pub fn normalize_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn option_texts(options: &[OptionItem]) -> Vec<(String, String)> {
    options
        .iter()
        .map(|o| (o.key.clone(), normalize_text(&o.text)))
        .collect()
}

fn answers_eq(a: &Answer, b: &Answer) -> bool {
    match (a, b) {
        (Answer::Single(x), Answer::Single(y)) => x == y,
        (Answer::Multiple(x), Answer::Multiple(y)) => x == y,
        (Answer::Judge(x), Answer::Judge(y)) => x == y,
        (Answer::Fill(x), Answer::Fill(y)) => {
            x.iter().map(|s| normalize_text(s)).collect::<Vec<_>>()
                == y.iter().map(|s| normalize_text(s)).collect::<Vec<_>>()
        }
        (Answer::Code(x), Answer::Code(y)) => normalize_text(x) == normalize_text(y),
        _ => false,
    }
}
