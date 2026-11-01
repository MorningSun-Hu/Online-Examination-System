use bank_html::{pretty_print, parse_html, Answer, OptionItem, QType, Question};
use proptest::prelude::*;

fn letter_strategy() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "A".to_string(),
        "B".to_string(),
        "C".to_string(),
        "D".to_string(),
    ])
}

fn stem_strategy() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 ]{3,20}"
}

prop_compose! {
    fn single_question()
        (stem in stem_strategy(), ans in letter_strategy())
        -> Question
    {
        let options = ["A", "B", "C", "D"]
            .into_iter()
            .map(|k| OptionItem { key: k.into(), text: format!("opt{k}") })
            .collect();
        Question {
            qtype: QType::Single,
            stem,
            stem_html: String::new(),
            options,
            answer: Answer::Single(ans),
            source: String::new(),
            paper_title: "试卷属性".into(),
        }
    }
}

proptest! {
    #[test]
    fn pretty_print_then_parse_keeps_semantics(q in single_question()) {
        let html = pretty_print("属性样例", &[q.clone()]);
        let report = parse_html(&html, "prop.html").expect("parse");
        prop_assert_eq!(report.success.len(), 1);
        prop_assert!(q.semantic_eq(&report.success[0]));
    }
}
