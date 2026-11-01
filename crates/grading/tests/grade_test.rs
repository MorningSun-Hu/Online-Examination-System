use bank_html::{Answer, QType};
use grading::{grade, StudentResponse};

#[test]
fn single_choice() {
    let out = grade(
        QType::Single,
        &Answer::Single("B".into()),
        &StudentResponse::Single("X".into()),
        5,
        Some(&["B".into()]),
    );
    assert_eq!(out.awarded, Some(5));
    let out = grade(
        QType::Single,
        &Answer::Single("B".into()),
        &StudentResponse::Single("A".into()),
        5,
        Some(&["A".into()]),
    );
    assert_eq!(out.awarded, Some(0));
}

#[test]
fn multiple_all_or_nothing() {
    let ans = Answer::Multiple(vec!["A".into(), "C".into()]);
    let ok = grade(
        QType::Multiple,
        &ans,
        &StudentResponse::Multiple(vec!["A".into(), "C".into()]),
        4,
        Some(&["A".into(), "C".into()]),
    );
    assert_eq!(ok.awarded, Some(4));
    let miss = grade(
        QType::Multiple,
        &ans,
        &StudentResponse::Multiple(vec!["A".into()]),
        4,
        Some(&["A".into()]),
    );
    assert_eq!(miss.awarded, Some(0));
}

#[test]
fn judge_and_code() {
    let j = grade(
        QType::Judge,
        &Answer::Judge(true),
        &StudentResponse::Judge(true),
        2,
        None,
    );
    assert_eq!(j.awarded, Some(2));
    let c = grade(
        QType::Code,
        &Answer::Code("print(1)".into()),
        &StudentResponse::Code("print(1)".into()),
        10,
        None,
    );
    assert_eq!(c.awarded, None);
}

#[test]
fn fill_alternatives() {
    let ans = Answer::Fill(vec!["len()".into(), "len".into()]);
    let out = grade(
        QType::Fill,
        &ans,
        &StudentResponse::Fill("len()".into()),
        3,
        None,
    );
    assert_eq!(out.awarded, Some(3));
}
