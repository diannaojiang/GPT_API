//! 思考内容格式归一化转换器。
//!
//! 将模型输出的“思考”/推理内容在三种格式之间互转：
//! - `ThinkTag`：`<think>...</think>` 包裹在 content 中
//! - `Reasoning`：独立的 `reasoning` 字段
//! - `ReasoningContent`：独立的 `reasoning_content` 字段
//!
//! 提供非流式（整段消息）与流式（逐 chunk 有状态）两套 API。

use crate::config::types::ThinkingFormat;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};

static THINK_BLOCK_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<think>(.*?)</think>").unwrap());

const OPEN_TAG: &str = "<think>";
const CLOSE_TAG: &str = "</think>";

/// 提取 content 中所有 `<think>...</think>` 块，返回 (拼接后的推理文本, 去除标签后的答案)。
/// 多个块之间以换行拼接；空块视为无推理。
fn extract_think_blocks(content: &str) -> (String, String) {
    let mut reasoning = String::new();
    for cap in THINK_BLOCK_RE.captures_iter(content) {
        if let Some(g) = cap.get(1) {
            let piece = g.as_str();
            if piece.is_empty() {
                continue;
            }
            if !reasoning.is_empty() {
                reasoning.push('\n');
            }
            reasoning.push_str(piece);
        }
    }
    let remainder = THINK_BLOCK_RE.replace_all(content, "").to_string();
    (reasoning, remainder)
}

/// content 字段的三种形态。
enum ContentKind {
    /// 字符串内容（已 clone）
    Str(String),
    /// null 或缺失
    Empty,
    /// 数组等其它类型（多模态）——不触碰
    Other,
}

fn classify_content(msg: &serde_json::Map<String, Value>) -> ContentKind {
    match msg.get("content") {
        Some(Value::String(s)) => ContentKind::Str(s.clone()),
        Some(Value::Null) | None => ContentKind::Empty,
        Some(_) => ContentKind::Other,
    }
}

fn field_reasoning(msg: &serde_json::Map<String, Value>) -> Option<String> {
    msg.get("reasoning")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            msg.get("reasoning_content")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        })
}

/// 对单个 assistant message 对象（choices[].message）做原地转换。
pub fn transform_message(msg: &mut Value, target: ThinkingFormat) {
    if matches!(target, ThinkingFormat::Passthrough) {
        return;
    }
    let Some(obj) = msg.as_object_mut() else {
        return;
    };

    let from_field = field_reasoning(obj);
    let kind = classify_content(obj);

    let (answer_string, extracted): (Option<String>, Option<String>) = match kind {
        ContentKind::Str(s) => {
            let (r, rem) = extract_think_blocks(&s);
            let ext = if r.is_empty() { None } else { Some(r) };
            (Some(rem), ext)
        }
        ContentKind::Empty => (Some(String::new()), None),
        ContentKind::Other => (None, None),
    };

    let source_reasoning = from_field.or(extracted).filter(|s| !s.is_empty());

    match target {
        ThinkingFormat::Reasoning | ThinkingFormat::ReasoningContent => {
            let tf = if matches!(target, ThinkingFormat::Reasoning) {
                "reasoning"
            } else {
                "reasoning_content"
            };
            obj.remove("reasoning");
            obj.remove("reasoning_content");
            if let Some(sr) = source_reasoning {
                obj.insert(tf.to_string(), Value::String(sr));
            }
            if let Some(ans) = answer_string {
                obj.insert("content".to_string(), Value::String(ans));
            }
        }
        ThinkingFormat::ThinkTag => {
            obj.remove("reasoning");
            obj.remove("reasoning_content");
            match (source_reasoning, answer_string) {
                (Some(sr), Some(ans)) => {
                    obj.insert(
                        "content".to_string(),
                        Value::String(format!("{}{}{}{}", OPEN_TAG, sr, CLOSE_TAG, ans)),
                    );
                }
                (Some(_), None) => {
                    // content 为数组等无法拼接的类型，保持原样，尽力而为丢弃推理字段。
                }
                (None, Some(ans)) => {
                    obj.insert("content".to_string(), Value::String(ans));
                }
                (None, None) => {}
            }
        }
        ThinkingFormat::Passthrough => {}
    }
}

/// 对完整的非流式 chat completion 响应体做转换，遍历 choices[].message。
pub fn transform_response_body(body: &mut Value, target: ThinkingFormat) {
    if matches!(target, ThinkingFormat::Passthrough) {
        return;
    }
    if let Some(choices) = body.get_mut("choices").and_then(|c| c.as_array_mut()) {
        for choice in choices.iter_mut() {
            if let Some(msg) = choice.get_mut("message") {
                transform_message(msg, target);
            }
        }
    }
}

/// content 解析阶段（用于 think-tag 源的流式提取）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// 正常内容
    Content,
    /// 处于 `<think>` 与 `</think>` 之间
    InThink,
}

/// 流式思考格式转换器。
///
/// 关键难点：`<think>`/`</think>` 标记可能跨 SSE chunk 断裂。通过 `carry` 保留
/// 上一 chunk 末尾可能构成标记前缀的字节，与下一 chunk 拼接后再扫描，从而正确
/// 识别被切分的标记。
pub struct ThinkingStreamTransformer {
    target: ThinkingFormat,
    /// think-tag 源解析阶段（Reasoning/ReasoningContent 目标下使用）
    phase: Phase,
    /// 跨 chunk 的标记前缀滑动缓冲
    carry: String,
    /// ThinkTag 目标：是否已输出开启标记 `<think>`
    think_open: bool,
    /// ThinkTag 目标：是否已输出闭合标记 `</think>`
    think_closed: bool,
}

/// 返回 `s` 的最长后缀长度，使其等于 `marker` 的某个真前缀（用于跨 chunk 标记检测）。
fn partial_marker_len(s: &str, marker: &str) -> usize {
    let sb = s.as_bytes();
    let mb = marker.as_bytes();
    let maxk = sb.len().min(mb.len().saturating_sub(1));
    for k in (1..=maxk).rev() {
        if sb[sb.len() - k..] == mb[..k] {
            return k;
        }
    }
    0
}

impl ThinkingStreamTransformer {
    pub fn new(target: ThinkingFormat) -> Self {
        Self {
            target,
            phase: Phase::Content,
            carry: String::new(),
            think_open: false,
            think_closed: false,
        }
    }

    /// 解析一段 content 片段（think-tag 源），返回 (输出到 content 的文本, 输出到推理字段的文本)。
    fn parse_content_source(&mut self, fragment: &str) -> (String, String) {
        let mut combined = std::mem::take(&mut self.carry);
        combined.push_str(fragment);
        let mut content_out = String::new();
        let mut reasoning_out = String::new();
        let mut rest = combined.as_str();

        loop {
            match self.phase {
                Phase::Content => {
                    if let Some(i) = rest.find(OPEN_TAG) {
                        content_out.push_str(&rest[..i]);
                        rest = &rest[i + OPEN_TAG.len()..];
                        self.phase = Phase::InThink;
                    } else {
                        let k = partial_marker_len(rest, OPEN_TAG);
                        let cut = rest.len() - k;
                        content_out.push_str(&rest[..cut]);
                        self.carry = rest[cut..].to_string();
                        break;
                    }
                }
                Phase::InThink => {
                    if let Some(j) = rest.find(CLOSE_TAG) {
                        reasoning_out.push_str(&rest[..j]);
                        rest = &rest[j + CLOSE_TAG.len()..];
                        self.phase = Phase::Content;
                    } else {
                        let k = partial_marker_len(rest, CLOSE_TAG);
                        let cut = rest.len() - k;
                        reasoning_out.push_str(&rest[..cut]);
                        self.carry = rest[cut..].to_string();
                        break;
                    }
                }
            }
        }
        (content_out, reasoning_out)
    }

    /// 对单个 delta 对象（choices[0].delta）做原地转换。
    pub fn transform_delta(&mut self, delta: &mut Value) {
        if matches!(self.target, ThinkingFormat::Passthrough) {
            return;
        }
        let Some(obj) = delta.as_object_mut() else {
            return;
        };

        match self.target {
            ThinkingFormat::Reasoning | ThinkingFormat::ReasoningContent => {
                let tf = if matches!(self.target, ThinkingFormat::Reasoning) {
                    "reasoning"
                } else {
                    "reasoning_content"
                };

                let dr = obj
                    .get("reasoning")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
                let drc = obj
                    .get("reasoning_content")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());

                if dr.is_some() || drc.is_some() {
                    let val = dr.or(drc).unwrap_or_default();
                    obj.remove("reasoning");
                    obj.remove("reasoning_content");
                    obj.insert(tf.to_string(), Value::String(val));
                    return;
                }

                let content_owned = obj
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                if let Some(content) = content_owned {
                    let (c_out, r_out) = self.parse_content_source(&content);
                    obj.insert("content".to_string(), Value::String(c_out));
                    if !r_out.is_empty() {
                        obj.insert(tf.to_string(), Value::String(r_out));
                    }
                }
            }
            ThinkingFormat::ThinkTag => {
                let dr = obj
                    .get("reasoning")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .or_else(|| {
                        obj.get("reasoning_content")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                    });
                let dc = obj
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let had_reasoning = dr.is_some();
                let had_content = dc.is_some();

                obj.remove("reasoning");
                obj.remove("reasoning_content");

                let mut out = String::new();
                if let Some(r) = dr {
                    if !self.think_open {
                        out.push_str(OPEN_TAG);
                        self.think_open = true;
                    }
                    out.push_str(&r);
                }
                if let Some(c) = dc {
                    if !c.is_empty() {
                        if self.think_open && !self.think_closed {
                            out.push_str(CLOSE_TAG);
                            self.think_closed = true;
                        }
                        out.push_str(&c);
                    }
                }
                if had_reasoning || had_content {
                    obj.insert("content".to_string(), Value::String(out));
                }
            }
            ThinkingFormat::Passthrough => {}
        }
    }

    /// 流结束时调用，返回需补发的 delta（若有），用于刷新缓冲状态。
    pub fn finalize(&mut self) -> Option<Value> {
        match self.target {
            ThinkingFormat::ThinkTag => {
                if self.think_open && !self.think_closed {
                    self.think_closed = true;
                    Some(json!({ "content": CLOSE_TAG }))
                } else {
                    None
                }
            }
            ThinkingFormat::Reasoning | ThinkingFormat::ReasoningContent => {
                if self.carry.is_empty() {
                    return None;
                }
                let leftover = std::mem::take(&mut self.carry);
                match self.phase {
                    Phase::Content => Some(json!({ "content": leftover })),
                    Phase::InThink => {
                        let tf = if matches!(self.target, ThinkingFormat::Reasoning) {
                            "reasoning"
                        } else {
                            "reasoning_content"
                        };
                        Some(json!({ tf: leftover }))
                    }
                }
            }
            ThinkingFormat::Passthrough => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -------------------------------------------------------------------------
    // Helper utilities for tests
    // -------------------------------------------------------------------------

    /// Run a vector of delta objects through a transformer and collect results.
    /// Note: delta is the object at choices[0].delta (e.g. {"content": "fragment"})
    fn run_streaming_transform(
        deltas: Vec<Value>,
        target: ThinkingFormat,
    ) -> (String, String, String) {
        let mut transformer = ThinkingStreamTransformer::new(target);
        let mut content_accum = String::new();
        let mut reasoning_accum = String::new();
        let mut reasoning_content_accum = String::new();

        for mut delta in deltas {
            transformer.transform_delta(&mut delta);

            if let Some(obj) = delta.as_object() {
                if let Some(c) = obj.get("content").and_then(|v| v.as_str()) {
                    content_accum.push_str(c);
                }
                if let Some(r) = obj.get("reasoning").and_then(|v| v.as_str()) {
                    reasoning_accum.push_str(r);
                }
                if let Some(rc) = obj.get("reasoning_content").and_then(|v| v.as_str()) {
                    reasoning_content_accum.push_str(rc);
                }
            }
        }

        if let Some(flush) = transformer.finalize() {
            if let Some(obj) = flush.as_object() {
                if let Some(c) = obj.get("content").and_then(|v| v.as_str()) {
                    content_accum.push_str(c);
                }
                if let Some(r) = obj.get("reasoning").and_then(|v| v.as_str()) {
                    reasoning_accum.push_str(r);
                }
                if let Some(rc) = obj.get("reasoning_content").and_then(|v| v.as_str()) {
                    reasoning_content_accum.push_str(rc);
                }
            }
        }

        (content_accum, reasoning_accum, reasoning_content_accum)
    }

    // -------------------------------------------------------------------------
    // Passthrough tests (non-streaming)
    // -------------------------------------------------------------------------

    #[test]
    fn test_passthrough_no_op_message() {
        let mut msg = json!({
            "content": "Hello",
            "reasoning": "thinking...",
            "reasoning_content": null
        });
        let original = msg.clone();
        transform_message(&mut msg, ThinkingFormat::Passthrough);
        assert_eq!(msg, original);
    }

    #[test]
    fn test_passthrough_no_op_response_body() {
        let mut body = json!({
            "choices": [
                {"message": {"content": "Hello", "reasoning": "thinking..."}}
            ]
        });
        let original = body.clone();
        transform_response_body(&mut body, ThinkingFormat::Passthrough);
        assert_eq!(body, original);
    }

    #[test]
    fn test_passthrough_preserves_all_fields() {
        let mut msg = json!({
            "content": "Answer",
            "reasoning": "Reasoning",
            "reasoning_content": "AltReasoning"
        });
        transform_message(&mut msg, ThinkingFormat::Passthrough);
        assert_eq!(msg["content"], "Answer");
        assert_eq!(msg["reasoning"], "Reasoning");
        assert_eq!(msg["reasoning_content"], "AltReasoning");
    }

    // -------------------------------------------------------------------------
    // Non-streaming: ThinkTag → Reasoning / ReasoningContent
    // -------------------------------------------------------------------------

    #[test]
    fn test_nonstreaming_think_tag_to_reasoning() {
        let mut msg = json!({
            "content": "<think>Let me think.</think>The answer is 42."
        });
        transform_message(&mut msg, ThinkingFormat::Reasoning);
        assert_eq!(msg["reasoning"], "Let me think.");
        assert_eq!(msg["content"], "The answer is 42.");
        assert!(!msg.as_object().unwrap().contains_key("reasoning_content"));
    }

    #[test]
    fn test_nonstreaming_think_tag_to_reasoning_content() {
        let mut msg = json!({
            "content": "<think>Let me think.</think>The answer is 42."
        });
        transform_message(&mut msg, ThinkingFormat::ReasoningContent);
        assert_eq!(msg["reasoning_content"], "Let me think.");
        assert_eq!(msg["content"], "The answer is 42.");
        assert!(!msg.as_object().unwrap().contains_key("reasoning"));
    }

    #[test]
    fn test_nonstreaming_think_tag_to_reasoning_multiline() {
        let mut msg = json!({
            "content": "<think>Line 1\nLine 2\nLine 3</think>Answer"
        });
        transform_message(&mut msg, ThinkingFormat::Reasoning);
        assert_eq!(msg["reasoning"], "Line 1\nLine 2\nLine 3");
        assert_eq!(msg["content"], "Answer");
    }

    #[test]
    fn test_nonstreaming_think_tag_to_reasoning_strips_all_tags() {
        let mut msg = json!({
            "content": "<think>First</think>Answer1<think>Second</think>Answer2"
        });
        transform_message(&mut msg, ThinkingFormat::Reasoning);
        // Both think tags should be extracted
        assert!(msg["reasoning"].as_str().unwrap().contains("First"));
        assert!(msg["reasoning"].as_str().unwrap().contains("Second"));
        assert!(msg["content"].as_str().unwrap().contains("Answer1"));
        assert!(msg["content"].as_str().unwrap().contains("Answer2"));
    }

    // -------------------------------------------------------------------------
    // Non-streaming: Reasoning → ThinkTag / ReasoningContent
    // -------------------------------------------------------------------------

    #[test]
    fn test_nonstreaming_reasoning_to_think_tag() {
        let mut msg = json!({
            "content": "The answer is 42.",
            "reasoning": "Let me think."
        });
        transform_message(&mut msg, ThinkingFormat::ThinkTag);
        assert_eq!(
            msg["content"],
            "<think>Let me think.</think>The answer is 42."
        );
        assert!(!msg.as_object().unwrap().contains_key("reasoning"));
    }

    #[test]
    fn test_nonstreaming_reasoning_to_reasoning_content() {
        let mut msg = json!({
            "content": "The answer is 42.",
            "reasoning": "Let me think."
        });
        transform_message(&mut msg, ThinkingFormat::ReasoningContent);
        assert_eq!(msg["reasoning_content"], "Let me think.");
        assert_eq!(msg["content"], "The answer is 42.");
        assert!(!msg.as_object().unwrap().contains_key("reasoning"));
    }

    #[test]
    fn test_nonstreaming_reasoning_to_think_tag_empty_reasoning() {
        let mut msg = json!({
            "content": "Just answer.",
            "reasoning": ""
        });
        transform_message(&mut msg, ThinkingFormat::ThinkTag);
        assert_eq!(msg["content"], "Just answer.");
        assert!(!msg.as_object().unwrap().contains_key("reasoning"));
    }

    // -------------------------------------------------------------------------
    // Non-streaming: ReasoningContent → ThinkTag / Reasoning
    // -------------------------------------------------------------------------

    #[test]
    fn test_nonstreaming_reasoning_content_to_think_tag() {
        let mut msg = json!({
            "content": "The answer is 42.",
            "reasoning_content": "Let me think."
        });
        transform_message(&mut msg, ThinkingFormat::ThinkTag);
        assert_eq!(
            msg["content"],
            "<think>Let me think.</think>The answer is 42."
        );
        assert!(!msg.as_object().unwrap().contains_key("reasoning_content"));
    }

    #[test]
    fn test_nonstreaming_reasoning_content_to_reasoning() {
        let mut msg = json!({
            "content": "The answer is 42.",
            "reasoning_content": "Let me think."
        });
        transform_message(&mut msg, ThinkingFormat::Reasoning);
        assert_eq!(msg["reasoning"], "Let me think.");
        assert_eq!(msg["content"], "The answer is 42.");
        assert!(!msg.as_object().unwrap().contains_key("reasoning_content"));
    }

    // -------------------------------------------------------------------------
    // Non-streaming: Cross-field conversions (reasoning ↔ reasoning_content)
    // -------------------------------------------------------------------------

    #[test]
    fn test_nonstreaming_reasoning_to_reasoning_content_field_rename() {
        let mut msg = json!({
            "content": "Answer",
            "reasoning": "Some reasoning"
        });
        transform_message(&mut msg, ThinkingFormat::ReasoningContent);
        assert_eq!(msg["reasoning_content"], "Some reasoning");
        assert_eq!(msg["content"], "Answer");
        assert!(!msg.as_object().unwrap().contains_key("reasoning"));
    }

    #[test]
    fn test_nonstreaming_reasoning_content_to_reasoning_field_rename() {
        let mut msg = json!({
            "content": "Answer",
            "reasoning_content": "Some reasoning"
        });
        transform_message(&mut msg, ThinkingFormat::Reasoning);
        assert_eq!(msg["reasoning"], "Some reasoning");
        assert_eq!(msg["content"], "Answer");
        assert!(!msg.as_object().unwrap().contains_key("reasoning_content"));
    }

    // -------------------------------------------------------------------------
    // Non-streaming: Edge cases
    // -------------------------------------------------------------------------

    #[test]
    fn test_nonstreaming_content_null() {
        let mut msg = json!({
            "content": null,
            "reasoning": "thinking"
        });
        transform_message(&mut msg, ThinkingFormat::Reasoning);
        assert_eq!(msg["reasoning"], "thinking");
        assert_eq!(msg["content"], "");
    }

    #[test]
    fn test_nonstreaming_content_absent() {
        let mut msg = json!({
            "reasoning": "thinking"
        });
        transform_message(&mut msg, ThinkingFormat::Reasoning);
        assert_eq!(msg["reasoning"], "thinking");
        assert_eq!(msg["content"], "");
    }

    #[test]
    fn test_nonstreaming_content_array_multimodal() {
        // Content is a JSON array (multimodal) — must not panic and not extract tags
        let mut msg = json!({
            "content": [{"type": "image_url", "image_url": {"url": "..."}}],
            "reasoning": "thinking"
        });
        transform_message(&mut msg, ThinkingFormat::Reasoning);
        // Should not crash and should move reasoning field
        assert_eq!(msg["reasoning"], "thinking");
        // Content should remain unchanged (array)
        assert!(msg["content"].is_array());
    }

    #[test]
    fn test_nonstreaming_no_think_tag_plain_content() {
        // Plain content without think tags stays intact
        let mut msg = json!({
            "content": "Just a plain answer."
        });
        transform_message(&mut msg, ThinkingFormat::Reasoning);
        assert!(!msg.as_object().unwrap().contains_key("reasoning"));
        assert_eq!(msg["content"], "Just a plain answer.");
    }

    #[test]
    fn test_nonstreaming_empty_reasoning() {
        let mut msg = json!({
            "content": "<think></think>Answer",
            "reasoning": ""
        });
        transform_message(&mut msg, ThinkingFormat::Reasoning);
        // Empty reasoning should not add the field
        assert!(
            !msg.as_object().unwrap().contains_key("reasoning")
                || msg["reasoning"]
                    .as_str()
                    .map(|s| s.is_empty())
                    .unwrap_or(false)
        );
        assert_eq!(msg["content"], "Answer");
    }

    #[test]
    fn test_nonstreaming_empty_content_with_reasoning() {
        let mut msg = json!({
            "content": "",
            "reasoning": "thinking"
        });
        transform_message(&mut msg, ThinkingFormat::ThinkTag);
        assert_eq!(msg["content"], "<think>thinking</think>");
    }

    // -------------------------------------------------------------------------
    // Non-streaming: Response body transformation
    // -------------------------------------------------------------------------

    #[test]
    fn test_nonstreaming_response_body_multiple_choices() {
        let mut body = json!({
            "choices": [
                {"message": {"content": "<think>R1</think>A1"}},
                {"message": {"content": "<think>R2</think>A2"}}
            ]
        });
        transform_response_body(&mut body, ThinkingFormat::Reasoning);
        assert_eq!(body["choices"][0]["message"]["reasoning"], "R1");
        assert_eq!(body["choices"][0]["message"]["content"], "A1");
        assert_eq!(body["choices"][1]["message"]["reasoning"], "R2");
        assert_eq!(body["choices"][1]["message"]["content"], "A2");
    }

    // -------------------------------------------------------------------------
    // Passthrough tests (streaming)
    // -------------------------------------------------------------------------

    #[test]
    fn test_streaming_passthrough_no_op() {
        let deltas = vec![json!({"content": "Hello", "reasoning": "thinking"})];
        let (content, reasoning, _reasoning_content) =
            run_streaming_transform(deltas, ThinkingFormat::Passthrough);
        assert_eq!(content, "Hello");
        assert_eq!(reasoning, "thinking");
    }

    #[test]
    fn test_streaming_passthrough_fields_untouched() {
        let deltas =
            vec![json!({"content": "Hi", "reasoning": "think", "reasoning_content": "alt"})];
        let (content, reasoning, reasoning_content) =
            run_streaming_transform(deltas, ThinkingFormat::Passthrough);
        assert_eq!(content, "Hi");
        assert_eq!(reasoning, "think");
        assert_eq!(reasoning_content, "alt");
    }

    // -------------------------------------------------------------------------
    // Streaming: ThinkTag → Reasoning with split markers
    // -------------------------------------------------------------------------

    #[test]
    fn test_streaming_think_tag_to_reasoning_split_markers() {
        // Feed: "<thi", "nk>reason", "ing</thi", "nk>ans", "wer"
        let deltas = vec![
            json!({"content": "<thi"}),
            json!({"content": "nk>reason"}),
            json!({"content": "ing</thi"}),
            json!({"content": "nk>ans"}),
            json!({"content": "wer"}),
        ];
        let (content, reasoning, _) = run_streaming_transform(deltas, ThinkingFormat::Reasoning);

        assert_eq!(content, "answer");
        assert_eq!(reasoning, "reasoning");
    }

    #[test]
    fn test_streaming_think_tag_to_reasoning_content_split_markers() {
        // "<thi"+"nk>reason"+"ing</thi"+"nk>answer" => <think>reasoning</think>answer
        let deltas = vec![
            json!({"content": "<thi"}),
            json!({"content": "nk>reason"}),
            json!({"content": "ing</thi"}),
            json!({"content": "nk>answer"}),
        ];
        let (content, _, reasoning_content) =
            run_streaming_transform(deltas, ThinkingFormat::ReasoningContent);

        assert_eq!(content, "answer");
        assert_eq!(reasoning_content, "reasoning");
    }

    // -------------------------------------------------------------------------
    // Streaming: Reasoning → ThinkTag
    // -------------------------------------------------------------------------

    #[test]
    fn test_streaming_reasoning_to_think_tag_multiple_chunks() {
        // First 2 chunks: reasoning fragments, Next 2 chunks: content fragments
        let deltas = vec![
            json!({"reasoning": "First part "}),
            json!({"reasoning": "second part "}),
            json!({"content": "Answer "}),
            json!({"content": "here"}),
        ];
        let (content, _, _) = run_streaming_transform(deltas, ThinkingFormat::ThinkTag);

        // Should have opening tag, reasoning, closing tag, answer
        assert!(content.starts_with("<think>"));
        assert!(content.contains("First part second part"));
        assert!(content.contains("</think>"));
        assert!(content.contains("Answer here"));
    }

    #[test]
    fn test_streaming_reasoning_to_think_tag_finalize_closing_tag() {
        // Reasoning-only (no content) — finalize should emit closing tag
        let deltas = vec![json!({"reasoning": "Only reasoning"})];
        let (content, _, _) = run_streaming_transform(deltas, ThinkingFormat::ThinkTag);

        assert!(content.contains("<think>Only reasoning</think>"));
    }

    #[test]
    fn test_streaming_reasoning_to_think_tag_empty_reasoning() {
        // Empty reasoning fragment
        let deltas = vec![json!({"reasoning": "", "content": "Just content"})];
        let (content, _, _) = run_streaming_transform(deltas, ThinkingFormat::ThinkTag);

        assert!(!content.contains("<think>")); // No opening since no reasoning
        assert_eq!(content, "Just content");
    }

    // -------------------------------------------------------------------------
    // Streaming: Reasoning → ReasoningContent (field rename)
    // -------------------------------------------------------------------------

    #[test]
    fn test_streaming_reasoning_to_reasoning_content_field_rename() {
        let deltas = vec![json!({"reasoning": "Reasoning text", "content": "Answer"})];
        let (content, _, reasoning_content) =
            run_streaming_transform(deltas, ThinkingFormat::ReasoningContent);

        assert_eq!(content, "Answer");
        assert_eq!(reasoning_content, "Reasoning text");
    }

    #[test]
    fn test_streaming_reasoning_to_reasoning_content_multiple_chunks() {
        let deltas = vec![
            json!({"reasoning": "Part1 "}),
            json!({"reasoning": "Part2", "content": "Answer"}),
        ];
        let (content, _, reasoning_content) =
            run_streaming_transform(deltas, ThinkingFormat::ReasoningContent);

        assert_eq!(content, "Answer");
        assert_eq!(reasoning_content, "Part1 Part2");
    }

    #[test]
    fn test_streaming_reasoning_content_to_reasoning_field_rename() {
        let deltas = vec![json!({"reasoning_content": "Alt reasoning", "content": "Answer"})];
        let (content, reasoning, _) = run_streaming_transform(deltas, ThinkingFormat::Reasoning);

        assert_eq!(content, "Answer");
        assert_eq!(reasoning, "Alt reasoning");
    }

    // -------------------------------------------------------------------------
    // Streaming: ThinkTag → ThinkTag (passthrough with carry guard)
    // -------------------------------------------------------------------------

    #[test]
    fn test_streaming_think_tag_to_think_tag_passthrough() {
        let deltas = vec![json!({"content": "<think>Think content</think>Answer"})];
        let (content, _, _) = run_streaming_transform(deltas, ThinkingFormat::ThinkTag);

        assert_eq!(content, "<think>Think content</think>Answer");
    }

    // -------------------------------------------------------------------------
    // Streaming: Edge cases
    // -------------------------------------------------------------------------

    #[test]
    fn test_streaming_empty_delta() {
        let deltas = vec![json!({})];
        let (content, reasoning, _) = run_streaming_transform(deltas, ThinkingFormat::Reasoning);

        assert_eq!(content, "");
        assert_eq!(reasoning, "");
    }

    #[test]
    fn test_streaming_delta_with_both_reasoning_fields() {
        // When both reasoning fields present, prefer `reasoning`
        let deltas = vec![
            json!({"reasoning": "Primary", "reasoning_content": "Secondary", "content": "Answer"}),
        ];
        let (content, reasoning, _) = run_streaming_transform(deltas, ThinkingFormat::Reasoning);

        assert_eq!(content, "Answer");
        assert_eq!(reasoning, "Primary");
    }

    #[test]
    fn test_streaming_finalize_no_extra_when_closed() {
        let deltas = vec![json!({"reasoning": "Think", "content": "Answer"})];
        let mut transformer = ThinkingStreamTransformer::new(ThinkingFormat::ThinkTag);

        for mut delta in deltas {
            transformer.transform_delta(&mut delta);
        }

        let flush = transformer.finalize();
        // Since we had content, no flush should be needed
        assert!(flush.is_none());
    }

    #[test]
    fn test_streaming_finalize_emits_close_tag_when_reasoning_only() {
        let deltas = vec![json!({"reasoning": "Only reasoning"})];
        let mut transformer = ThinkingStreamTransformer::new(ThinkingFormat::ThinkTag);

        for mut delta in deltas {
            transformer.transform_delta(&mut delta);
        }

        let flush = transformer.finalize();
        assert!(flush.is_some());
        let flush_val = flush.unwrap();
        let flush_obj = flush_val.as_object().unwrap();
        assert_eq!(
            flush_obj.get("content").and_then(|v| v.as_str()),
            Some("</think>")
        );
    }
}
