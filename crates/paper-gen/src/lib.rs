use std::collections::HashMap;

use bank_html::{OptionItem, QType, Question};
use rand::seq::SliceRandom;
use rand::Rng;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GenError {
    #[error("{qtype:?} 现有 {available} 题，规则要求 {required} 题")]
    NotEnough {
        qtype: QType,
        available: usize,
        required: usize,
    },
    #[error("总分 {total} 必须大于等于题量 {count}")]
    TotalLessThanCount { total: u32, count: usize },
}

#[derive(Debug, Clone)]
pub struct PaperRule {
    pub type_counts: HashMap<QType, usize>,
    pub shuffle_within_section: bool,
    pub shuffle_options: bool,
    pub total_score: u32,
    pub type_subtotals: Option<HashMap<QType, u32>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperItem {
    pub source_index: usize,
    pub qtype: QType,
    pub option_map: Vec<(String, String)>,
    pub points: u32,
    pub options: Vec<OptionItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaperSnapshot {
    pub items: Vec<PaperItem>,
}

pub const TYPE_ORDER: [QType; 5] = [
    QType::Single,
    QType::Multiple,
    QType::Judge,
    QType::Fill,
    QType::Code,
];

pub fn generate<R: Rng>(
    questions: &[Question],
    rule: &PaperRule,
    rng: &mut R,
) -> Result<PaperSnapshot, GenError> {
    let mut grouped: HashMap<QType, Vec<usize>> = HashMap::new();
    for (i, q) in questions.iter().enumerate() {
        grouped.entry(q.qtype).or_default().push(i);
    }

    let mut picked: Vec<(QType, usize)> = Vec::new();
    for qtype in TYPE_ORDER {
        let need = match rule.type_counts.get(&qtype) {
            Some(&n) if n > 0 => n,
            _ => continue,
        };
        let pool = grouped.get(&qtype).cloned().unwrap_or_default();
        if pool.len() < need {
            return Err(GenError::NotEnough {
                qtype,
                available: pool.len(),
                required: need,
            });
        }
        let mut pool = pool;
        pool.shuffle(rng);
        pool.truncate(need);
        if rule.shuffle_within_section {
            pool.shuffle(rng);
        } else {
            pool.sort_unstable();
        }
        for idx in pool {
            picked.push((qtype, idx));
        }
    }

    let plan = allocate_scores(rule, &picked)?;
    let mut items = Vec::with_capacity(picked.len());
    for (i, (qtype, source_index)) in picked.into_iter().enumerate() {
        let q = &questions[source_index];
        let (options, option_map) = if rule.shuffle_options && !q.options.is_empty() {
            shuffle_options(&q.options, rng)
        } else {
            let map = q
                .options
                .iter()
                .map(|o| (o.key.clone(), o.key.clone()))
                .collect();
            (q.options.clone(), map)
        };
        items.push(PaperItem {
            source_index,
            qtype,
            option_map,
            points: plan[i],
            options,
        });
    }
    Ok(PaperSnapshot { items })
}

fn shuffle_options<R: Rng>(
    options: &[OptionItem],
    rng: &mut R,
) -> (Vec<OptionItem>, Vec<(String, String)>) {
    let mut order: Vec<usize> = (0..options.len()).collect();
    order.shuffle(rng);
    let keys: Vec<String> = options.iter().map(|o| o.key.clone()).collect();
    let mut mapped = Vec::new();
    let mut new_opts = Vec::new();
    for (i, old_i) in order.into_iter().enumerate() {
        let new_key = keys.get(i).cloned().unwrap_or_else(|| ((b'A' + i as u8) as char).to_string());
        let old_key = options[old_i].key.clone();
        mapped.push((new_key.clone(), old_key));
        new_opts.push(OptionItem {
            key: new_key,
            text: options[old_i].text.clone(),
        });
    }
    (new_opts, mapped)
}

pub fn allocate_scores(rule: &PaperRule, picked: &[(QType, usize)]) -> Result<Vec<u32>, GenError> {
    let n = picked.len();
    let t = rule.total_score;
    if (t as usize) < n {
        return Err(GenError::TotalLessThanCount { total: t, count: n });
    }
    let mut counts: HashMap<QType, usize> = HashMap::new();
    for (qtype, _) in picked {
        *counts.entry(*qtype).or_default() += 1;
    }

    let mut subtotals: HashMap<QType, u32> = HashMap::new();
    if let Some(given) = &rule.type_subtotals {
        let sum: u32 = TYPE_ORDER.iter().filter_map(|k| given.get(k)).sum();
        if sum == t {
            for qtype in TYPE_ORDER {
                if let Some(&s) = given.get(&qtype) {
                    subtotals.insert(qtype, s);
                }
            }
        } else if sum > 0 {
            let mut scaled: Vec<(QType, u32)> = TYPE_ORDER
                .iter()
                .filter_map(|k| given.get(k).map(|s| (*k, ((*s as u64 * t as u64) + (sum as u64 / 2)) / sum as u64)))
                .map(|(k, v)| (k, v as u32))
                .collect();
            let mut ssum: u32 = scaled.iter().map(|(_, v)| *v).sum();
            let mut i = 0;
            let nscaled = scaled.len();
            while ssum != t && !scaled.is_empty() {
                if ssum < t {
                    scaled[i % nscaled].1 += 1;
                    ssum += 1;
                } else if scaled[i % nscaled].1 > 0 {
                    scaled[i % nscaled].1 -= 1;
                    ssum -= 1;
                }
                i += 1;
                if i > 10_000 {
                    break;
                }
            }
            for (k, v) in scaled {
                subtotals.insert(k, v);
            }
        }
    }

    if subtotals.is_empty() {
        let mut remaining = t;
        let types_present: Vec<QType> = TYPE_ORDER
            .iter()
            .copied()
            .filter(|k| counts.get(k).copied().unwrap_or(0) > 0)
            .collect();
        for (i, qtype) in types_present.iter().enumerate() {
            let c = counts[qtype] as u32;
            if i + 1 == types_present.len() {
                subtotals.insert(*qtype, remaining);
            } else {
                let share = t * c / n as u32;
                subtotals.insert(*qtype, share);
                remaining = remaining.saturating_sub(share);
            }
        }
    }

    let mut per_type: HashMap<QType, Vec<u32>> = HashMap::new();
    for qtype in TYPE_ORDER {
        let c = match counts.get(&qtype).copied() {
            Some(c) if c > 0 => c,
            _ => continue,
        };
        let s = *subtotals.get(&qtype).unwrap_or(&0);
        let q = s / c as u32;
        let r = s % c as u32;
        let mut pts = vec![q; c];
        for item in pts.iter_mut().take(r as usize) {
            *item += 1;
        }
        per_type.insert(qtype, pts);
    }

    let mut cursor: HashMap<QType, usize> = HashMap::new();
    let mut plan = Vec::with_capacity(n);
    for (qtype, _) in picked {
        let i = cursor.entry(*qtype).or_default();
        let pts = per_type.get(qtype).and_then(|v| v.get(*i)).copied().unwrap_or(0);
        *i += 1;
        plan.push(pts);
    }

    while plan.iter().any(|&p| p == 0) {
        let zero = plan.iter().position(|&p| p == 0).unwrap();
        let donor = plan
            .iter()
            .enumerate()
            .max_by_key(|(_, p)| *p)
            .map(|(i, _)| i)
            .unwrap();
        if plan[donor] <= 1 {
            break;
        }
        plan[donor] -= 1;
        plan[zero] += 1;
    }
    Ok(plan)
}

pub fn restore_original_keys(exam_keys: &[String], option_map: &[(String, String)]) -> Vec<String> {
    exam_keys
        .iter()
        .filter_map(|k| {
            option_map
                .iter()
                .find(|(exam, _)| exam == k)
                .map(|(_, orig)| orig.clone())
        })
        .collect()
}
