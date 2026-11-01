use std::collections::BTreeSet;

use bank_html::{Answer, QType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StudentResponse {
    Single(String),
    Multiple(Vec<String>),
    Judge(bool),
    Fill(String),
    Code(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradeOutcome {
    pub awarded: Option<u32>,
    pub correct: Option<bool>,
}

pub fn grade(
    qtype: QType,
    answer: &Answer,
    response: &StudentResponse,
    points: u32,
    original_keys: Option<&[String]>,
) -> GradeOutcome {
    match qtype {
        QType::Code => GradeOutcome {
            awarded: None,
            correct: None,
        },
        QType::Single => {
            let expect = match answer {
                Answer::Single(k) => k.as_str(),
                _ => {
                    return GradeOutcome {
                        awarded: Some(0),
                        correct: Some(false),
                    }
                }
            };
            let got = match response {
                StudentResponse::Single(k) => original_keys
                    .and_then(|keys| keys.first().map(|s| s.as_str()))
                    .unwrap_or(k.as_str()),
                _ => "",
            };
            let ok = got == expect;
            GradeOutcome {
                awarded: Some(if ok { points } else { 0 }),
                correct: Some(ok),
            }
        }
        QType::Multiple => {
            let expect: BTreeSet<String> = match answer {
                Answer::Multiple(ks) => ks.iter().cloned().collect(),
                _ => BTreeSet::new(),
            };
            let got: BTreeSet<String> = match response {
                StudentResponse::Multiple(_) => original_keys
                    .map(|k| k.iter().cloned().collect())
                    .unwrap_or_default(),
                _ => BTreeSet::new(),
            };
            let ok = expect == got;
            GradeOutcome {
                awarded: Some(if ok { points } else { 0 }),
                correct: Some(ok),
            }
        }
        QType::Judge => {
            let expect = match answer {
                Answer::Judge(v) => *v,
                _ => false,
            };
            let got = match response {
                StudentResponse::Judge(v) => *v,
                _ => !expect,
            };
            let ok = expect == got;
            GradeOutcome {
                awarded: Some(if ok { points } else { 0 }),
                correct: Some(ok),
            }
        }
        QType::Fill => {
            let expects = match answer {
                Answer::Fill(vs) => vs.clone(),
                _ => Vec::new(),
            };
            let got = match response {
                StudentResponse::Fill(s) => s.as_str(),
                _ => "",
            };
            let got_n = normalize_fill(got);
            let ok = expects.iter().any(|e| normalize_fill(e) == got_n);
            GradeOutcome {
                awarded: Some(if ok { points } else { 0 }),
                correct: Some(ok),
            }
        }
    }
}

pub fn normalize_fill(s: &str) -> String {
    let half: String = s
        .chars()
        .map(|c| {
            if c == '\u{3000}' {
                ' '
            } else if ('\u{FF01}'..='\u{FF5E}').contains(&c) {
                char::from_u32(c as u32 - 0xFEE0).unwrap_or(c)
            } else {
                c
            }
        })
        .collect();
    half.split_whitespace().collect::<Vec<_>>().join(" ")
}
