#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivationMode {
    Continuous,
    Wake,
}

impl ActivationMode {
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("wake") => Self::Wake,
            _ => Self::Continuous,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActivationDecision {
    pub accepted: bool,
    pub reason: &'static str,
}

pub fn evaluate(transcript: &str, follow_up_window_open: bool) -> ActivationDecision {
    let normalized = transcript.trim().to_lowercase().replace(
        [' ', '\t', '\n', '，', '。', '！', '？', ',', '.', '!', '?'],
        "",
    );

    if normalized.is_empty() {
        return ActivationDecision {
            accepted: false,
            reason: "empty_transcript",
        };
    }
    if ["进入静默", "静默模式", "先这样", "不用了", "你先别说话"]
        .iter()
        .any(|command| normalized.contains(command))
    {
        return ActivationDecision {
            accepted: false,
            reason: "sleep_command",
        };
    }
    if follow_up_window_open {
        return ActivationDecision {
            accepted: true,
            reason: "follow_up_window",
        };
    }
    if ["ripple", "瑞波", "瑞普", "睿普"]
        .iter()
        .any(|wake_word| normalized.contains(wake_word))
    {
        return ActivationDecision {
            accepted: true,
            reason: "wake_word",
        };
    }
    if [
        "帮我记住",
        "请记住",
        "记住这个",
        "记一下这个",
        "记录一下",
        "记录这个",
        "保存这个",
        "保存一下",
        "拍一下",
        "拍下来",
        "做成待办",
        "创建待办",
        "加个待办",
        "记个待办",
        "加入待办",
        "提醒我",
    ]
    .iter()
    .any(|command| normalized.contains(command))
    {
        return ActivationDecision {
            accepted: true,
            reason: "explicit_command",
        };
    }
    ActivationDecision {
        accepted: false,
        reason: "not_addressed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_words_and_explicit_capture_commands_activate() {
        assert_eq!(evaluate("Ripple，帮我看看这里", false).reason, "wake_word");
        assert_eq!(evaluate("把这个记录一下", false).reason, "explicit_command");
        assert!(evaluate("提醒我明天检查阀门", false).accepted);
    }

    #[test]
    fn unrelated_background_speech_stays_silent() {
        assert_eq!(
            evaluate("今天晚上我们出去吃饭吧", false),
            ActivationDecision {
                accepted: false,
                reason: "not_addressed",
            }
        );
    }

    #[test]
    fn follow_up_window_accepts_without_repeating_the_name() {
        assert_eq!(evaluate("再看一下左边", true).reason, "follow_up_window");
        assert_eq!(evaluate("先这样吧", true).reason, "sleep_command");
    }
}
