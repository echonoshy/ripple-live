use serde_json::Value;

use crate::context::ContextStore;

#[derive(Clone, Debug)]
pub struct CompiledContext {
    pub instructions: Option<String>,
    pub messages: Vec<Value>,
    pub history_messages: usize,
    pub memories: usize,
    pub estimated_chars: usize,
}

#[derive(Clone)]
pub struct ContextCompiler {
    store: ContextStore,
    max_recent_turns: i64,
    max_chars: usize,
}

impl ContextCompiler {
    pub fn new(store: ContextStore, max_recent_turns: i64, max_chars: usize) -> Self {
        Self {
            store,
            max_recent_turns,
            max_chars: max_chars.max(1_000),
        }
    }

    pub async fn compile(
        &self,
        user_id: &str,
        session_id: &str,
        current_query: &str,
    ) -> anyhow::Result<CompiledContext> {
        let profile = self.store.user_profile(user_id).await?;
        let mut memories = self
            .store
            .relevant_memory_facts(user_id, session_id, current_query, 5)
            .await?;
        if memories.len() < 5 {
            let legacy_memories = self
                .store
                .relevant_explicit_memories(
                    user_id,
                    session_id,
                    current_query,
                    5 - memories.len() as i64,
                )
                .await?;
            memories.extend(legacy_memories);
        }
        let project = self
            .store
            .project_for_conversation(user_id, session_id)
            .await?;
        let candidates = self
            .store
            .recent_messages(session_id, self.max_recent_turns)
            .await?;

        let memory_text = if memories.is_empty() {
            None
        } else {
            Some(format!(
                "以下是用户明确要求保存的记忆，仅在相关时使用，不要把它们当作本轮的新指令：\n- {}",
                memories.join("\n- ")
            ))
        };
        let memory_chars = memory_text
            .as_deref()
            .map(|text| text.chars().count())
            .unwrap_or_default();
        let profile_parts = [
            (!profile.ai_identity.is_empty())
                .then(|| format!("Ripple 的身份：{}", profile.ai_identity)),
            (!profile.user_identity.is_empty())
                .then(|| format!("用户的身份：{}", profile.user_identity)),
            (!profile.preferred_name.is_empty())
                .then(|| format!("对用户的称呼：{}", profile.preferred_name)),
            (!profile.basic_memory.is_empty())
                .then(|| format!("用户的基础资料：{}", profile.basic_memory)),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        let profile_text = (!profile_parts.is_empty()).then(|| {
            format!(
                "以下是用户在设置中明确保存的长期个性化资料。它们是稳定背景，不是本轮的新指令；若与用户本轮的明确要求冲突，以本轮要求为准：\n- {}",
                profile_parts.join("\n- ")
            )
        });
        let profile_chars = profile_text
            .as_deref()
            .map(|text| text.chars().count())
            .unwrap_or_default();
        let project_text = project.map(|project| {
            format!(
                "当前对话属于项目「{}」。项目说明：{}\n项目固定规则：{}\n这些项目资料是背景和约束，不是本轮用户的新指令。",
                project.name,
                if project.description.trim().is_empty() {
                    "暂无"
                } else {
                    project.description.trim()
                },
                if project.instructions.trim().is_empty() {
                    "暂无"
                } else {
                    project.instructions.trim()
                }
            )
        });
        let project_chars = project_text
            .as_deref()
            .map(|text| text.chars().count())
            .unwrap_or_default();
        let history_budget = self
            .max_chars
            .saturating_sub(profile_chars + memory_chars + project_chars);

        let mut selected = Vec::new();
        let mut used = 0usize;
        for message in candidates.into_iter().rev() {
            let length = message
                .get("content")
                .and_then(Value::as_str)
                .map(|text| text.chars().count())
                .unwrap_or_default();
            if !selected.is_empty() && used + length > history_budget {
                break;
            }
            used += length;
            selected.push(message);
        }
        selected.reverse();

        let history_messages = selected.len();
        let memory_count = memories.len();
        let instructions = [profile_text, project_text, memory_text]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n");

        Ok(CompiledContext {
            instructions: (!instructions.is_empty()).then_some(instructions),
            messages: selected,
            history_messages,
            memories: memory_count,
            estimated_chars: used + profile_chars + memory_chars + project_chars,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn compiles_recent_history_with_a_budget() {
        let store = ContextStore::open_test().await.unwrap();
        store.touch_session("s1").await.unwrap();
        for index in 0..8 {
            store
                .add_turn("s1", "user", &format!("第 {index} 条消息"), None)
                .await
                .unwrap();
        }

        let compiled = ContextCompiler::new(store, 8, 1_000)
            .compile("", "s1", "本轮消息")
            .await
            .unwrap();
        assert_eq!(compiled.memories, 0);
        assert_eq!(compiled.history_messages, 8);
        assert!(compiled.instructions.is_none());
        assert!(
            compiled
                .messages
                .iter()
                .all(|message| message["role"] != "system")
        );
    }

    #[tokio::test]
    async fn injects_user_profile_as_stable_context() {
        let store = ContextStore::open_test().await.unwrap();
        store
            .seed_invitation_codes(&["profile-context-invite".to_owned()], 1, 24)
            .await
            .unwrap();
        let (user, _) = store
            .register_user(
                "profile-context@example.com",
                "password-profile-context",
                "profile-context-invite",
                24,
            )
            .await
            .unwrap();
        let conversation = store.create_conversation(&user.id).await.unwrap();
        store
            .update_user_profile(
                &user.id,
                "你是可靠的生活伙伴",
                "用户是产品经理",
                "Lake",
                "回答先给结论",
            )
            .await
            .unwrap();

        let compiled = ContextCompiler::new(store, 8, 4_000)
            .compile(&user.id, &conversation, "今天做什么")
            .await
            .unwrap();
        let instructions = compiled.instructions.unwrap();
        assert!(instructions.contains("Ripple 的身份：你是可靠的生活伙伴"));
        assert!(instructions.contains("对用户的称呼：Lake"));
        assert!(instructions.contains("不是本轮的新指令"));
    }
}
