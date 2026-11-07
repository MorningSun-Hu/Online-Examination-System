use std::collections::HashMap;

use bank_html::{Answer, OptionItem, QType, Question};
use paper_gen::{generate, map_to_exam_keys, restore_original_keys, PaperRule, TYPE_ORDER};
use rand::rngs::StdRng;
use rand::SeedableRng;

fn q(qtype: QType, i: usize) -> Question {
    let options = if matches!(qtype, QType::Single | QType::Multiple) {
        vec!["A", "B", "C", "D"]
            .into_iter()
            .map(|k| OptionItem {
                key: k.into(),
                text: format!("{k}{i}"),
            })
            .collect()
    } else {
        vec![]
    };
    let answer = match qtype {
        QType::Single => Answer::Single("B".into()),
        QType::Multiple => Answer::Multiple(vec!["A".into(), "C".into()]),
        QType::Judge => Answer::Judge(true),
        QType::Fill => Answer::Fill(vec!["x".into()]),
        QType::Code => Answer::Code("print(1)".into()),
    };
    Question {
        qtype,
        stem: format!("stem{i}"),
        stem_html: String::new(),
        options,
        answer,
        source: String::new(),
        paper_title: String::new(),
    }
}

fn pool() -> Vec<Question> {
    let mut v = Vec::new();
    for i in 0..10 {
        v.push(q(QType::Single, i));
    }
    for i in 0..5 {
        v.push(q(QType::Judge, i + 10));
    }
    v
}

#[test]
fn not_enough_questions() {
    let questions = pool();
    let mut type_counts = HashMap::new();
    type_counts.insert(QType::Single, 20);
    let rule = PaperRule {
        type_counts,
        shuffle_within_section: false,
        shuffle_options: false,
        total_score: 20,
        type_subtotals: None,
    };
    let mut rng = StdRng::seed_from_u64(1);
    assert!(generate(&questions, &rule, &mut rng).is_err());
}

#[test]
fn fixed_seed_and_score_sum() {
    let questions = pool();
    let mut type_counts = HashMap::new();
    type_counts.insert(QType::Single, 4);
    type_counts.insert(QType::Judge, 2);
    let rule = PaperRule {
        type_counts,
        shuffle_within_section: true,
        shuffle_options: true,
        total_score: 100,
        type_subtotals: None,
    };
    let mut rng = StdRng::seed_from_u64(42);
    let snap = generate(&questions, &rule, &mut rng).unwrap();
    assert_eq!(snap.items.len(), 6);
    assert_eq!(snap.items.iter().map(|i| i.points).sum::<u32>(), 100);
    assert!(snap.items.iter().all(|i| i.points >= 1));
    let _ = TYPE_ORDER;
}

#[test]
fn option_map_restores_original_answers() {
    let questions = pool();
    let mut type_counts = HashMap::new();
    type_counts.insert(QType::Single, 3);
    let rule = PaperRule {
        type_counts,
        shuffle_within_section: true,
        shuffle_options: true,
        total_score: 9,
        type_subtotals: None,
    };
    let mut rng = StdRng::seed_from_u64(7);
    let snap = generate(&questions, &rule, &mut rng).unwrap();
    for item in snap.items {
        let q = &questions[item.source_index];
        if let Answer::Single(orig) = &q.answer {
            let exam_key = item
                .option_map
                .iter()
                .find(|(_, o)| o == orig)
                .map(|(e, _)| e.clone())
                .unwrap();
            let restored = restore_original_keys(&[exam_key], &item.option_map);
            assert_eq!(restored, vec![orig.clone()]);
        }
    }
}

#[test]
fn map_to_exam_keys_follows_display_order() {
    let map = vec![
        ("A".into(), "C".into()),
        ("B".into(), "A".into()),
        ("C".into(), "B".into()),
    ];
    assert_eq!(map_to_exam_keys(&["A".into()], &map), vec!["B".to_string()]);
    assert_eq!(
        map_to_exam_keys(&["A".into(), "C".into()], &map),
        vec!["A".to_string(), "B".to_string()]
    );
    assert_eq!(map_to_exam_keys(&["A".into()], &[]), vec!["A".to_string()]);
}
