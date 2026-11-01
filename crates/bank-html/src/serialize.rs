use crate::types::{Answer, QType, Question};

const CSS: &str = r#"
:root { color-scheme: light; }
html, body, h1, h2, h3, p, li, div, pre, small, strong, span, ol, ul, hr {
    color: #000;
}
body {
    font-family: "SimSun", "Songti SC", serif;
    line-height: 1.8;
    max-width: 800px;
    margin: 2em auto;
    padding: 0 1em;
    background: #fff;
}
h1, h2, h3, strong {
    font-family: "SimHei", "Microsoft YaHei", "PingFang SC", sans-serif;
    font-weight: bold;
}
h1 { font-size: 1.4em; text-align: center; border-bottom: 1px solid #000; padding-bottom: 0.3em; }
h2 { font-size: 1.2em; margin-top: 2.2em; border-bottom: 1px solid #000; padding-bottom: 0.2em; }
h3 { font-size: 1.05em; margin-top: 1.5em; }
pre {
    font-family: "Courier New", monospace;
    border: 1px solid #000;
    padding: 0.5em;
    white-space: pre-wrap;
}
.question { margin: 0.8em 0; }
.options { margin: 0.2em 0 0.2em 1.5em; }
.answer { margin: 0.2em 0 0.8em 1.5em; font-weight: bold; }
.check-mark { font-family: "Segoe UI Symbol", "Arial Unicode MS", "DejaVu Sans", Arial, sans-serif; font-weight: normal; }
"#;

pub fn pretty_print(title: &str, questions: &[Question]) -> String {
    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html lang=\"zh-CN\">\n<head>\n");
    out.push_str("    <meta charset=\"UTF-8\">\n");
    out.push_str("    <title>");
    out.push_str(&escape(title));
    out.push_str("</title>\n    <style>");
    out.push_str(CSS);
    out.push_str("    </style>\n</head>\n<body>\n\n");
    out.push_str("<h1>");
    out.push_str(&escape(title));
    out.push_str("</h1>\n");

    let mut last_paper = String::from("__none__");
    let mut last_type: Option<QType> = None;
    let mut type_index = 0usize;
    for (i, q) in questions.iter().enumerate() {
        if q.paper_title != last_paper {
            last_paper = q.paper_title.clone();
            last_type = None;
            if !q.paper_title.is_empty() {
                out.push_str("\n<h2>");
                out.push_str(&escape(&q.paper_title));
                out.push_str("</h2>\n");
            }
        }
        if last_type != Some(q.qtype) {
            type_index += 1;
            last_type = Some(q.qtype);
            out.push_str("\n<h3>");
            out.push_str(&section_heading(type_index, q.qtype));
            out.push_str("</h3>\n");
        }
        out.push_str(&format_question(i + 1, q));
    }
    out.push_str("\n</body>\n</html>\n");
    out
}

fn format_question(n: usize, q: &Question) -> String {
    let mut s = String::from("\n<div class=\"question\">\n");
    if q.stem_html.trim().is_empty() {
        s.push_str("    <p>第");
        s.push_str(&n.to_string());
        s.push_str("题 ");
        s.push_str(&escape(&q.stem));
        s.push_str("</p>\n");
    } else if q.stem_html.contains('<') {
        s.push_str("    ");
        s.push_str(&q.stem_html);
        s.push('\n');
    } else {
        s.push_str("    <p>第");
        s.push_str(&n.to_string());
        s.push_str("题 ");
        s.push_str(&escape(&q.stem));
        s.push_str("</p>\n");
    }
    if !q.options.is_empty() {
        s.push_str("    <div class=\"options\">");
        let parts: Vec<String> = q
            .options
            .iter()
            .map(|o| format!("{}. {}", o.key, escape(&o.text)))
            .collect();
        s.push_str(&parts.join("　"));
        s.push_str("</div>\n");
    }
    s.push_str("    <div class=\"answer\">");
    s.push_str(&format_answer(q));
    s.push_str("</div>\n</div>\n");
    s
}

fn format_answer(q: &Question) -> String {
    match &q.answer {
        Answer::Single(k) => format!("答案：{}", k),
        Answer::Multiple(ks) => format!("答案：{}", ks.join("")),
        Answer::Judge(true) => {
            "答案：<span class=\"check-mark\">&#10003;</span>".to_string()
        }
        Answer::Judge(false) => "答案：×".to_string(),
        Answer::Fill(vs) => format!("答案：{}", vs.join("|")),
        Answer::Code(code) => format!("参考答案：<br><pre>{}</pre>", escape(code)),
    }
}

fn section_heading(idx: usize, qtype: QType) -> String {
    let name = match qtype {
        QType::Single | QType::Multiple => "选择题",
        QType::Judge => "判断题",
        QType::Fill => "填空题",
        QType::Code => "编程题",
    };
    format!("{}、{}", chinese_index(idx), name)
}

fn chinese_index(n: usize) -> &'static str {
    match n {
        1 => "一",
        2 => "二",
        3 => "三",
        4 => "四",
        5 => "五",
        _ => "六",
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
