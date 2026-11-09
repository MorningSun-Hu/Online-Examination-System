use bank_html::{parse_html, pretty_print, Answer, QType};

fn read(path: &str) -> String {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("testdata/bank-html");
    p.push(path);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

#[test]
fn parse_spec_sample() {
    let html = read("spec-sample.html");
    let report = parse_html(&html, "spec-sample.html").expect("parse");
    assert_eq!(report.success.len(), 5);
    assert!(report.failures.is_empty());
    assert_eq!(report.success[0].qtype, QType::Single);
    assert_eq!(report.success[0].answer, Answer::Single("B".into()));
    assert_eq!(report.success[1].qtype, QType::Multiple);
    assert_eq!(
        report.success[1].answer,
        Answer::Multiple(vec!["A".into(), "B".into(), "D".into()])
    );
    assert_eq!(report.success[2].qtype, QType::Judge);
    assert_eq!(report.success[2].answer, Answer::Judge(false));
    assert_eq!(report.success[3].qtype, QType::Fill);
    assert_eq!(
        report.success[3].answer,
        Answer::Fill(vec!["len()".into(), "len".into()])
    );
    assert_eq!(report.success[4].qtype, QType::Code);
}

#[test]
fn whole_file_fails_when_empty() {
    let html = read("empty.html");
    assert!(parse_html(&html, "empty.html").is_err());
}

#[test]
fn partial_import_keeps_good_questions() {
    let html = read("partial.html");
    let report = parse_html(&html, "partial.html").expect("parse");
    assert_eq!(report.success.len(), 1);
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].index, 2);
}

#[test]
fn parse_python_basics_200() {
    let html = read("python-basics-200.html");
    let report = parse_html(&html, "python-basics-200.html").expect("parse");
    assert!(report.failures.is_empty(), "{:?}", report.failures);
    assert_eq!(report.success.len(), 200);
    let singles = report
        .success
        .iter()
        .filter(|q| q.qtype == QType::Single)
        .count();
    let judges = report
        .success
        .iter()
        .filter(|q| q.qtype == QType::Judge)
        .count();
    let fills = report
        .success
        .iter()
        .filter(|q| q.qtype == QType::Fill)
        .count();
    assert_eq!(singles, 150);
    assert_eq!(judges, 20);
    assert_eq!(fills, 30);
}

#[test]
fn fill_keeps_ascii_parentheses() {
    let html = read("python-basics-200.html");
    let report = parse_html(&html, "python-basics-200.html").expect("parse");
    let q = report
        .success
        .iter()
        .find(|q| q.stem.contains("1~100 随机整数"))
        .expect("randint fill");
    assert_eq!(
        q.answer,
        Answer::Fill(vec!["random.randint(1, 100)".into()])
    );
    let hello = report
        .success
        .iter()
        .find(|q| q.stem.contains("输出 Hello"))
        .expect("print fill");
    assert_eq!(hello.answer, Answer::Fill(vec!["print(\"Hello\")".into()]));
    let flt = report
        .success
        .iter()
        .find(|q| q.stem.contains("3.14"))
        .expect("float fill");
    assert_eq!(flt.answer, Answer::Fill(vec!["float(\"3.14\")".into()]));
}

#[test]
fn fill_or_and_enumeration_are_alternatives() {
    let html = r#"<!DOCTYPE html><html><body>
<h2>三、填空题</h2>
<ol>
<li>print(________)<br>答案：i ** 2（或 i * i）</li>
<li>另一空 ________<br>答案：foo、bar</li>
</ol>
</body></html>"#;
    let report = parse_html(html, "fill-alt.html").expect("parse");
    assert_eq!(report.success.len(), 2);
    assert_eq!(
        report.success[0].answer,
        Answer::Fill(vec!["i ** 2".into(), "i * i".into()])
    );
    assert_eq!(
        report.success[1].answer,
        Answer::Fill(vec!["foo".into(), "bar".into()])
    );
}

#[test]
fn comparison_operator_in_code_sample_stays_intact() {
    let html = read("python-basics-200.html");
    let report = parse_html(&html, "python-basics-200.html").expect("parse");
    let q = report
        .success
        .iter()
        .find(|q| q.stem.contains(r#"print("a<b")"#))
        .expect("comparison question");
    assert!(
        q.stem.contains(r#"下一行 print("结束") 与 if 对齐"#),
        "stem truncated: {}",
        q.stem
    );
    let opts: Vec<(&str, &str)> = q
        .options
        .iter()
        .map(|o| (o.key.as_str(), o.text.as_str()))
        .collect();
    assert_eq!(
        opts,
        vec![
            ("A", "只输出 a<b"),
            ("B", "只输出 结束"),
            ("C", "两行都输出"),
            ("D", "报错"),
        ]
    );
    assert_eq!(q.answer, Answer::Single("C".into()));
    assert!(
        q.stem_html.contains("a&lt;b"),
        "stem_html should keep escaped lt: {}",
        q.stem_html
    );
    assert!(
        !q.stem_html.contains("A."),
        "stem_html should drop inline options: {}",
        q.stem_html
    );
}

#[test]
fn round_trip_spec_sample() {
    let html = read("spec-sample.html");
    let report = parse_html(&html, "spec-sample.html").unwrap();
    let out = pretty_print(&report.title, &report.success);
    let again = parse_html(&out, "round.html").unwrap();
    assert_eq!(again.success.len(), report.success.len());
    for (a, b) in report.success.iter().zip(again.success.iter()) {
        assert!(a.semantic_eq(b), "{:?} vs {:?}", a, b);
    }
}
