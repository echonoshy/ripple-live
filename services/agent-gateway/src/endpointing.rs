use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointDecision {
    Complete,
    Continue,
    Uncertain,
}

impl EndpointDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Continue => "continue",
            Self::Uncertain => "uncertain",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EndpointEvaluation {
    pub transcript: String,
    pub decision: EndpointDecision,
    pub reason: &'static str,
    pub classifier_latency_ms: Option<u128>,
}

/// Normalizes a potential spoken command without changing its semantic content.
pub fn normalize_command(text: &str) -> String {
    let mut command: String = text
        .chars()
        .filter(|character| !character.is_whitespace() && !is_separator(*character))
        .flat_map(char::to_lowercase)
        .collect();

    const WAKE_NAMES: &[&str] = &["瑞波", "ripple"];
    const POLITE_FILLERS: &[&str] = &["麻烦你", "请你", "你先", "麻烦", "请", "你"];

    if let Some(wake_name) = WAKE_NAMES
        .iter()
        .find(|wake_name| command.starts_with(**wake_name))
    {
        command = command[wake_name.len()..].to_owned();
    }

    while let Some(filler) = POLITE_FILLERS
        .iter()
        .find(|filler| command.starts_with(**filler))
    {
        command = command[filler.len()..].to_owned();
    }

    command
}

pub fn is_stop_command(text: &str) -> bool {
    let command = normalize_command(text);
    [
        "停",
        "停下",
        "停一下",
        "停止",
        "别说了",
        "不要说了",
        "不用说了",
        "先别说",
        "安静",
    ]
    .iter()
    .any(|candidate| command == *candidate)
}

/// Returns a decision only when the transcript has an unambiguous ending.
pub fn deterministic_decision(text: &str) -> Option<EndpointDecision> {
    let normalized = normalize_command(text);
    if normalized.is_empty() {
        return None;
    }

    const INCOMPLETE_ENDINGS: &[&str] = &[
        "然后",
        "但是",
        "因为",
        "如果",
        "我想",
        "我觉得",
        "这个",
        "那个",
    ];
    if INCOMPLETE_ENDINGS
        .iter()
        .any(|ending| normalized.ends_with(ending))
    {
        return Some(EndpointDecision::Continue);
    }

    let trimmed = text.trim_end();
    if trimmed.ends_with(['?', '？'])
        || normalized.ends_with(['吗', '呢', '吧', '呀', '啊'])
        || ["开始", "继续", "暂停", "停止", "取消", "安静"]
            .iter()
            .any(|command| normalized == *command)
    {
        return Some(EndpointDecision::Complete);
    }

    None
}

pub fn parse_classifier_decision(text: &str) -> Option<(EndpointDecision, f32)> {
    let value: Value = serde_json::from_str(text)
        .or_else(|_| serde_json::from_str(&text.replace("\\\"", "\"")))
        .ok()?;
    let decision = match value.get("decision")?.as_str()? {
        "complete" => EndpointDecision::Complete,
        "continue" => EndpointDecision::Continue,
        _ => return None,
    };
    let confidence = value.get("confidence")?.as_f64()? as f32;
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return None;
    }

    Some((decision, confidence))
}

fn is_separator(character: char) -> bool {
    character.is_ascii_punctuation()
        || matches!(
            character,
            '，' | '。'
                | '！'
                | '？'
                | '、'
                | '；'
                | '：'
                | '“'
                | '”'
                | '‘'
                | '’'
                | '（'
                | '）'
                | '【'
                | '】'
                | '《'
                | '》'
                | '…'
                | '—'
                | '－'
        )
}

#[cfg(test)]
mod tests {
    use super::{
        EndpointDecision, deterministic_decision, is_stop_command, normalize_command,
        parse_classifier_decision,
    };

    #[test]
    fn only_complete_stop_utterances_are_consumed() {
        assert!(is_stop_command("瑞波，停一下"));
        assert!(is_stop_command("你先不要说了"));
        assert!(!is_stop_command("停止计时"));
        assert!(!is_stop_command("不要说这个"));
    }

    #[test]
    fn normalization_removes_only_leading_wake_name_fillers_and_separators() {
        assert_eq!(normalize_command(" 瑞波， 请 停一下！ "), "停一下");
        assert_eq!(normalize_command("不要说这个"), "不要说这个");
    }

    #[test]
    fn incomplete_lead_ins_continue_but_finished_questions_complete() {
        assert_eq!(
            deterministic_decision("因为"),
            Some(EndpointDecision::Continue)
        );
        assert_eq!(
            deterministic_decision("我觉得"),
            Some(EndpointDecision::Continue)
        );
        assert_eq!(
            deterministic_decision("今天天气怎么样？"),
            Some(EndpointDecision::Complete)
        );
        assert_eq!(
            deterministic_decision("告诉我天气吧"),
            Some(EndpointDecision::Complete)
        );
        assert_eq!(deterministic_decision("今天天气"), None);
    }

    #[test]
    fn classifier_output_is_parsed_without_applying_confidence_policy() {
        assert_eq!(
            parse_classifier_decision(r#"{\"decision\":\"complete\",\"confidence\":0.91}"#),
            Some((EndpointDecision::Complete, 0.91))
        );
        assert_eq!(
            parse_classifier_decision(r#"{\"decision\":\"complete\",\"confidence\":0.5}"#),
            Some((EndpointDecision::Complete, 0.5))
        );
        assert_eq!(parse_classifier_decision("answer now"), None);
    }

    #[test]
    fn decision_names_match_the_wire_protocol() {
        assert_eq!(EndpointDecision::Complete.as_str(), "complete");
        assert_eq!(EndpointDecision::Continue.as_str(), "continue");
        assert_eq!(EndpointDecision::Uncertain.as_str(), "uncertain");
    }
}
