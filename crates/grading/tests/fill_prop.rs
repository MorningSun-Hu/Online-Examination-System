use bank_html::{Answer, QType};
use grading::{grade, normalize_fill, StudentResponse};
use proptest::prelude::*;

proptest! {
    #[test]
    fn fullwidth_halfwidth_same_verdict(s in "[A-Za-z0-9 ]{1,12}") {
        let wide: String = s
            .chars()
            .map(|c| {
                if c == ' ' {
                    '\u{3000}'
                } else {
                    char::from_u32(c as u32 + 0xFEE0).unwrap_or(c)
                }
            })
            .collect();
        let ans = Answer::Fill(vec![s.clone()]);
        let a = grade(
            QType::Fill,
            &ans,
            &StudentResponse::Fill(s.clone()),
            1,
            None,
        );
        let b = grade(
            QType::Fill,
            &ans,
            &StudentResponse::Fill(wide),
            1,
            None,
        );
        prop_assert_eq!(a.correct, b.correct);
        prop_assert_eq!(normalize_fill(&s), normalize_fill(&s));
    }
}
