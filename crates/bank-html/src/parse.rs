use regex::Regex;
use scraper::{Html, Selector};
use std::sync::OnceLock;

use crate::types::{
    Answer, ImportFailure, ImportReport, OptionItem, QType, Question, normalize_text,
};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("{filename}: {reason}")]
    WholeFile { filename: String, reason: String },
}

pub fn parse_html(html: &str, filename: &str) -> Result<ImportReport, ParseError> {
    let document = Html::parse_document(html);
    let title = text_of_first(&document, "h1").unwrap_or_else(|| filename.to_string());
    let mut success = Vec::new();
    let mut failures = Vec::new();

    let question_sel = Selector::parse("div.question").expect("selector");
    let questions: Vec<_> = document.select(&question_sel).collect();
    if !questions.is_empty() {
        let mut current_type = QType::Single;
        let mut paper_title = String::new();
        let h2_sel = Selector::parse("h2").expect("selector");
        let h3_sel = Selector::parse("h3").expect("selector");
        let blocks: Vec<_> = document
            .select(&Selector::parse("h2, h3, div.question").expect("selector"))
            .collect();
        for el in blocks {
            let name = el.value().name();
            if name == "h2" {
                paper_title = el.text().collect::<String>();
                paper_title = normalize_text(&paper_title);
            } else if name == "h3" {
                current_type = infer_type_from_heading(&el.text().collect::<String>());
            } else {
                let idx = success.len() + failures.len() + 1;
                match parse_question_div(el, current_type, &paper_title) {
                    Ok(q) => success.push(q),
                    Err(reason) => failures.push(ImportFailure {
                        index: idx,
                        reason,
                        excerpt: excerpt(&el.html()),
                    }),
                }
            }
        }
        let _ = h2_sel;
        let _ = h3_sel;
    } else {
        parse_list_document(&document, &title, &mut success, &mut failures);
    }

    if success.is_empty() && failures.is_empty() {
        return Err(ParseError::WholeFile {
            filename: filename.to_string(),
            reason: "无法按题库格式解析出题目（缺少 .question 或可识别的列表项）".into(),
        });
    }
    if success.is_empty() {
        return Err(ParseError::WholeFile {
            filename: filename.to_string(),
            reason: format!(
                "未能解析出题目。首条失败：{}",
                failures
                    .first()
                    .map(|f| f.reason.as_str())
                    .unwrap_or("未知")
            ),
        });
    }

    Ok(ImportReport {
        filename: filename.to_string(),
        title,
        success,
        failures,
    })
}

fn parse_list_document(
    document: &Html,
    _title: &str,
    success: &mut Vec<Question>,
    failures: &mut Vec<ImportFailure>,
) {
   let block_sel = Selector::parse("body > *").expect("selector");
    let li_sel = Selector::parse("li").expect("selector");
    let mut current_type = QType::Single;
    let mut paper_title = String::new();
    for el in document.select(&block_sel) {
        match el.value().name() {
            "h2" => {
                let text = normalize_text(&el.text().collect::<String>());
                current_type = infer_type_from_heading(&text);
                paper_title = text;
            }
            "ol" | "ul" => {
                for li in el.select(&li_sel) {
                    let idx = success.len() + failures.len() + 1;
                    match parse_list_item(li, current_type, &paper_title) {
                        Ok(q) => success.push(q),
                        Err(reason) => failures.push(ImportFailure {
                            index: idx,
                            reason,
                            excerpt: excerpt(&li.html()),
                        }),
                    }
                }
            }
            _ => {}
        }
    }
}

fn parse_question_div(
    el: scraper::ElementRef<'_>,
    heading_type: QType,
    paper_title: &str,
) -> Result<Question, String> {
    let html = el.html();
    let answer_sel = Selector::parse("div.answer").expect("selector");
    let options_sel = Selector::parse("div.options").expect("selector");
    let answer_el = el.select(&answer_sel).next();
    let answer_text = answer_el
        .map(|a| a.text().collect::<String>())
        .unwrap_or_default();
    let options_text = el
        .select(&options_sel)
        .next()
        .map(|o| o.text().collect::<String>())
        .unwrap_or_default();

    let mut stem_html = String::new();
    for child in el.children() {
        if let Some(child_el) = scraper::ElementRef::wrap(child) {
            let name = child_el.value().name();
            if name == "div" {
                let class = child_el.value().attr("class").unwrap_or("");
                if class.contains("answer") || class.contains("options") {
                    continue;
                }
            }
            stem_html.push_str(&child_el.html());
        }
    }
    let stem = normalize_text(&strip_tags(&stem_html));
    let stem = strip_question_no(&stem);
    if stem.is_empty() {
        return Err("题干为空".into());
    }
    let options = split_options(&options_text);
    let qtype = refine_type(heading_type, &answer_text, &options);
    let answer = parse_answer(qtype, &answer_text, &html)?;
    Ok(Question {
        qtype,
        stem,
        stem_html: stem_html.trim().to_string(),
        options,
        answer,
        source: String::new(),
        paper_title: paper_title.to_string(),
    })
}

fn parse_list_item(
    el: scraper::ElementRef<'_>,
    heading_type: QType,
    paper_title: &str,
) -> Result<Question, String> {
    let inner = inner_html(el);
    let (body, answer_raw) = split_answer_html(&inner).ok_or_else(|| "缺少答案".to_string())?;
    let full_text = normalize_text(&el.text().collect::<String>());
    let (body_text, answer_text) = split_answer_html(&full_text)
        .map(|(b, a)| (normalize_text(&b), normalize_text(&a)))
        .unwrap_or_else(|| {
            (
                normalize_text(&strip_tags(&body)),
                normalize_text(&strip_tags(&answer_raw)),
            )
        });
    let options = split_options(&body_text);
    let stem = if options.is_empty() {
        body_text
    } else {
        let cut = body_text.find("A.").unwrap_or(body_text.len());
        normalize_text(&body_text[..cut])
    };
    if stem.is_empty() {
        return Err("题干为空".into());
    }
    let qtype = refine_type(heading_type, &answer_text, &options);
    let answer = parse_answer(qtype, &answer_text, &answer_raw)?;
    Ok(Question {
        qtype,
        stem,
        stem_html: if options.is_empty() {
            body.trim().to_string()
        } else {
            strip_inline_options_html(body.trim())
        },
        options,
        answer,
        source: String::new(),
        paper_title: paper_title.to_string(),
    })
}

fn split_answer_html(inner: &str) -> Option<(String, String)> {
    if let Some(pos) = inner.find("答案：") {
        Some((inner[..pos].to_string(), inner[pos + "答案：".len()..].to_string()))
    } else if let Some(pos) = inner.find("参考答案：") {
        Some((
            inner[..pos].to_string(),
            inner[pos + "参考答案：".len()..].to_string(),
        ))
    } else {
        None
    }
}

fn refine_type(heading: QType, answer_text: &str, options: &[OptionItem]) -> QType {
    if heading == QType::Judge {
        return QType::Judge;
    }
    if heading == QType::Code {
        return QType::Code;
    }
    if heading == QType::Fill {
        return QType::Fill;
    }
    if !options.is_empty() {
        let letters = extract_answer_letters(answer_text);
        if letters.len() >= 2 {
            return QType::Multiple;
        }
        return QType::Single;
    }
    if is_judge_answer(answer_text) {
        return QType::Judge;
    }
    heading
}

fn parse_answer(qtype: QType, answer_text: &str, raw: &str) -> Result<Answer, String> {
    match qtype {
        QType::Single => {
            let letters = extract_answer_letters(answer_text);
            letters
                .into_iter()
                .next()
                .map(Answer::Single)
                .ok_or_else(|| "单选题缺少答案字母".into())
        }
        QType::Multiple => {
            let letters = extract_answer_letters(answer_text);
            if letters.is_empty() {
                Err("多选题缺少答案字母".into())
            } else {
                Ok(Answer::Multiple(letters))
            }
        }
        QType::Judge => parse_judge(answer_text).map(Answer::Judge),
        QType::Fill => Ok(Answer::Fill(split_fill_answers(answer_text))),
        QType::Code => {
            let code = extract_pre(raw).unwrap_or_else(|| answer_text.trim().to_string());
            Ok(Answer::Code(code))
        }
    }
}

fn parse_judge(text: &str) -> Result<bool, String> {
    let t = text.trim();
    if t.contains('✓') || t.contains('\u{2713}') || t.contains("正确") || t.eq_ignore_ascii_case("true")
    {
        return Ok(true);
    }
    if t.contains('×') || t.contains('✗') || t.contains("错误") || t.eq_ignore_ascii_case("false") {
        return Ok(false);
    }
    Err("判断题答案无法识别".into())
}

fn is_judge_answer(text: &str) -> bool {
    parse_judge(text).is_ok() && extract_answer_letters(text).is_empty()
}

fn extract_answer_letters(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for ch in text.chars() {
        if ch.is_ascii_uppercase() && ch <= 'H' {
            let s = ch.to_string();
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

fn split_fill_answers(text: &str) -> Vec<String> {
    let t = text.trim().trim_start_matches("答案：").trim();
    let mut parts: Vec<String> = Vec::new();
    for chunk in t.split('|') {
        for sub in chunk.split("或") {
            for piece in sub.split('、') {
                let cleaned = piece
                    .replace('（', " ")
                    .replace('）', " ")
                    .replace('“', "")
                    .replace('”', "");
                let v = normalize_text(&cleaned);
                if !v.is_empty() && v != "或" {
                    parts.push(v);
                }
            }
        }
    }
    if parts.is_empty() {
        parts.push(normalize_text(t));
    }
    parts
}

fn split_options(text: &str) -> Vec<OptionItem> {
    let re = option_re();
    let starts: Vec<(usize, String)> = re
        .captures_iter(text)
        .filter_map(|c| {
            let m = c.get(0)?;
            let key = c.get(1)?.as_str().to_string();
            Some((m.end(), key))
        })
        .collect();
    if starts.is_empty() {
        return Vec::new();
    }
    let caps: Vec<_> = re.find_iter(text).collect();
    let mut items = Vec::new();
    for (i, mat) in caps.iter().enumerate() {
        let key = re
            .captures(&text[mat.start()..])
            .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
            .unwrap_or_default();
        let from = mat.end();
        let to = caps.get(i + 1).map(|n| n.start()).unwrap_or(text.len());
        let body = normalize_text(&text[from..to]);
        if !key.is_empty() {
            items.push(OptionItem { key, text: body });
        }
    }
    let _ = starts;
    items
}

fn option_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"([A-H])\.\s*").expect("option regex"))
}

fn infer_type_from_heading(text: &str) -> QType {
    if text.contains("判断") {
        QType::Judge
    } else if text.contains("填空") {
        QType::Fill
    } else if text.contains("编程") {
        QType::Code
    } else {
        QType::Single
    }
}

fn text_of_first(document: &Html, sel: &str) -> Option<String> {
    let selector = Selector::parse(sel).ok()?;
    document
        .select(&selector)
        .next()
        .map(|e| normalize_text(&e.text().collect::<String>()))
}

fn strip_tags(html: &str) -> String {
    let frag = Html::parse_fragment(html);
    normalize_text(&frag.root_element().text().collect::<String>())
}

fn strip_question_no(stem: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^第\s*\d+\s*题\s*").expect("qn"));
    normalize_text(&re.replace(stem, ""))
}

fn inner_html(el: scraper::ElementRef<'_>) -> String {
    let mut s = String::new();
    for child in el.children() {
        if let Some(e) = scraper::ElementRef::wrap(child) {
            s.push_str(&e.html());
        } else if let Some(t) = child.value().as_text() {
            s.push_str(&escape_text(t));
        }
    }
    s
}

fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn strip_inline_options_html(html: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?i)<br\s*/?>\s*A\s*[.．]").expect("opt br"));
    if let Some(m) = re.find(html) {
        return html[..m.start()].trim_end().to_string();
    }
    html.trim().to_string()
}

fn extract_pre(html: &str) -> Option<String> {
    let frag = Html::parse_fragment(html);
    let sel = Selector::parse("pre").ok()?;
    frag.select(&sel)
        .next()
        .map(|e| e.text().collect::<String>())
}

fn excerpt(s: &str) -> String {
    let t = normalize_text(s);
    t.chars().take(80).collect()
}
