use std::collections::HashMap;

use bank_html::{Answer, QType, Question};
use paper_gen::{allocate_scores, PaperRule};
use proptest::prelude::*;

proptest! {
    #[test]
    fn points_positive_and_sum_to_total(total in 6u32..200, extra in 0u32..20) {
        let n_single = 4usize;
        let n_judge = 2usize;
        let n = n_single + n_judge;
        let t = total.max(n as u32) + extra;
        let mut type_counts = HashMap::new();
        type_counts.insert(QType::Single, n_single);
        type_counts.insert(QType::Judge, n_judge);
        let rule = PaperRule {
            type_counts,
            shuffle_within_section: false,
            shuffle_options: false,
            total_score: t,
            type_subtotals: None,
        };
        let mut picked = Vec::new();
        for i in 0..n_single {
            picked.push((QType::Single, i));
        }
        for i in 0..n_judge {
            picked.push((QType::Judge, i));
        }
        let plan = allocate_scores(&rule, &picked).unwrap();
        prop_assert_eq!(plan.iter().sum::<u32>(), t);
        prop_assert!(plan.iter().all(|&p| p >= 1));
        let _ = Question {
            qtype: QType::Single,
            stem: "s".into(),
            stem_html: String::new(),
            options: vec![],
            answer: Answer::Single("A".into()),
            source: String::new(),
            paper_title: String::new(),
        };
    }
}
