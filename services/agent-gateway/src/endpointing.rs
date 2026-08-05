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

pub fn normalize_command(text: &str) -> String {
    let mut command: String = text
        .chars()
        .filter(|character| !character.is_whitespace() && !is_separator(*character))
        .flat_map(char::to_lowercase)
        .collect();
    const WAKE_NAMES: &[&str] = &["瑞波", "ripple"];
    const POLITE_FILLERS: &[&str] = &["麻烦你", "请你", "你先", "麻烦", "请", "你"];
    if let Some(wake_name) = WAKE_NAMES.iter().find(|name| command.starts_with(**name)) {
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
        "停下来",
        "停下来吧",
        "停下吧",
        "先停下来",
        "先停下吧",
        "可以停了",
        "可以停下了",
        "可以停下来",
        "可以停下来了",
        "别再说了",
        "不要再说了",
        "不要继续说了",
        "不用继续说了",
        "停止回复",
        "停止回答",
        "进入静默状态",
        "进入静默模式",
        "进入静默",
        "进去静默状态",
        "进去静默模式",
        "进去静默",
        "静默状态",
        "静默模式",
    ]
    .iter()
    .any(|candidate| command == *candidate)
}

pub fn deterministic_decision(text: &str) -> Option<EndpointDecision> {
    let normalized = normalize_command(text);
    if normalized.is_empty() {
        return None;
    }
    let incomplete = [
        "然后",
        "但是",
        "因为",
        "如果",
        "我想",
        "我觉得",
        "这个",
        "那个",
    ];
    let without_terminal_particle = normalized.trim_end_matches(['吗', '呢', '吧', '呀', '啊']);
    if incomplete
        .iter()
        .any(|ending| without_terminal_particle.ends_with(ending))
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

pub fn parse_classifier_decision(text: &str) -> Option<(EndpointDecision, f64)> {
    let value: Value = serde_json::from_str(text)
        .or_else(|_| serde_json::from_str(&text.replace("\\\"", "\"")))
        .ok()?;
    let decision = match value.get("decision")?.as_str()? {
        "complete" => EndpointDecision::Complete,
        "continue" => EndpointDecision::Continue,
        _ => return None,
    };
    let confidence = value.get("confidence")?.as_f64()?;
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
        EndpointDecision, deterministic_decision, is_stop_command, parse_classifier_decision,
    };

    #[test]
    fn endpointing_primitives_distinguish_complete_continue_and_unknown() {
        assert!(is_stop_command("瑞波，停一下"));
        assert!(!is_stop_command("停止计时"));
        assert_eq!(
            deterministic_decision("因为"),
            Some(EndpointDecision::Continue)
        );
        assert_eq!(
            deterministic_decision("天气怎么样？"),
            Some(EndpointDecision::Complete)
        );
        assert_eq!(deterministic_decision("今天天气"), None);
        assert_eq!(
            parse_classifier_decision(r#"{\"decision\":\"complete\",\"confidence\":0.75}"#),
            Some((EndpointDecision::Complete, 0.75))
        );
    }

    #[test]
    fn silent_mode_phrases_are_handled_as_stop_commands() {
        for transcript in [
            "进入静默状态",
            "进入静默模式",
            "进入静默",
            "进去静默状态",
            "进去静默模式",
            "进去静默",
            "静默状态",
            "静默模式",
        ] {
            assert!(is_stop_command(transcript), "{transcript}");
        }
    }

    #[test]
    fn explicit_stop_variants_are_handled_as_stop_commands() {
        for transcript in [
            "停下来",
            "停下来吧",
            "停下吧",
            "先停下来",
            "先停下吧",
            "可以停了",
            "可以停下了",
            "可以停下来",
            "可以停下来了",
            "别再说了",
            "不要再说了",
            "不要继续说了",
            "不用继续说了",
            "停止回复",
            "停止回答",
        ] {
            assert!(is_stop_command(transcript), "{transcript}");
        }
    }

    #[test]
    fn stop_words_inside_larger_requests_are_not_intercepted() {
        for transcript in [
            "停下来之后继续讲",
            "停止计时",
            "别再说刚才那件事",
            "可以停在这里吗",
        ] {
            assert!(!is_stop_command(transcript), "{transcript}");
        }
    }
}
