use std::{env, net::SocketAddr, path::PathBuf, time::Duration};

#[derive(Clone, Debug)]
pub struct Settings {
    pub address: SocketAddr,
    pub data_dir: PathBuf,
    pub database_url: String,
    pub database_max_connections: u32,
    pub asr_backend: String,
    pub asr_url: String,
    pub asr_model: String,
    pub asr_readiness_url: String,
    pub agent_backend: String,
    pub agent_url: String,
    pub agent_model: String,
    pub agent_api_key: String,
    pub agent_readiness_url: String,
    pub agent_temperature: f64,
    pub agent_max_tokens: u32,
    pub tts_backend: String,
    pub tts_url: String,
    pub tts_model: String,
    pub tts_voice: String,
    pub tts_language: String,
    pub tts_instructions: String,
    pub tts_readiness_url: String,
    pub audio_chunk_ms: usize,
    pub sample_rate_in: u32,
    pub sample_rate_out: u32,
    pub max_audio_seconds: usize,
    pub max_frames: usize,
    pub max_recent_turns: i64,
    pub context_max_chars: usize,
    pub max_tool_rounds: usize,
    pub skills_dir: PathBuf,
    pub cli_max_output_bytes: usize,
    pub search_proxy: String,
    pub invite_codes: Vec<String>,
    pub invite_max_uses: i64,
    pub invite_ttl_hours: i64,
    pub auth_token_ttl_hours: i64,
    pub request_timeout: Duration,
    pub gate_timeout: Duration,
    pub readiness_timeout: Duration,
}

fn value(name: &str, default: &str) -> String {
    env::var(format!("RIPPLE_{name}")).unwrap_or_else(|_| default.to_owned())
}

fn parsed<T>(name: &str, default: T) -> T
where
    T: std::str::FromStr,
{
    env::var(format!("RIPPLE_{name}"))
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(default)
}

fn readiness_url(name: &str, service_url: &str, default_path: &str) -> String {
    let override_name = format!("RIPPLE_{name}_READINESS_URL");
    if let Ok(value) = env::var(override_name)
        && !value.trim().is_empty()
    {
        return value;
    }
    reqwest::Url::parse(service_url)
        .map(|mut url| {
            url.set_path(default_path);
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        })
        .unwrap_or_else(|_| service_url.to_owned())
}

impl Settings {
    pub fn from_env() -> anyhow::Result<Self> {
        let host = value("HOST", "0.0.0.0");
        let port: u16 = parsed("PORT", 8700);
        let asr_url = value("ASR_URL", "http://127.0.0.1:8711/v1/audio/transcriptions");
        let agent_url = value("AGENT_URL", "http://127.0.0.1:8712/v1/responses");
        let tts_url = value("TTS_URL", "http://127.0.0.1:8723/v1/audio/speech");
        Ok(Self {
            address: format!("{host}:{port}").parse()?,
            data_dir: PathBuf::from(value("DATA_DIR", "runtime-data/agent-gateway")),
            database_url: value(
                "DATABASE_URL",
                "postgres://ripple_live:ripple_live@127.0.0.1:5432/ripple_live",
            ),
            database_max_connections: parsed("DATABASE_MAX_CONNECTIONS", 16),
            asr_backend: value("ASR_BACKEND", "openai"),
            asr_readiness_url: readiness_url("ASR", &asr_url, "/health"),
            asr_url,
            asr_model: value("ASR_MODEL", "Qwen3-ASR-1.7B"),
            agent_backend: value("AGENT_BACKEND", "openai"),
            agent_readiness_url: readiness_url("AGENT", &agent_url, "/v1/models"),
            agent_url,
            agent_model: value("AGENT_MODEL", "Qwen3.5-35B-A3B"),
            agent_api_key: value("AGENT_API_KEY", "EMPTY"),
            agent_temperature: parsed("AGENT_TEMPERATURE", 0.2),
            agent_max_tokens: parsed("AGENT_MAX_TOKENS", 1024),
            tts_backend: value("TTS_BACKEND", "qwen"),
            tts_readiness_url: readiness_url("TTS", &tts_url, "/health"),
            tts_url,
            tts_model: value("TTS_MODEL", "Qwen3-TTS-12Hz-1.7B-CustomVoice"),
            tts_voice: value("TTS_VOICE", "serena"),
            tts_language: value("TTS_LANGUAGE", "Chinese"),
            tts_instructions: value(
                "TTS_INSTRUCTIONS",
                "自然、温暖、亲切的中文女声。语速偏快但吐字清晰，语气平静连贯，停顿简洁自然，保持稳定一致的音色，像真人助手交流，避免播音腔和夸张情绪。",
            ),
            audio_chunk_ms: parsed("AUDIO_CHUNK_MS", 100),
            sample_rate_in: parsed("SAMPLE_RATE_IN", 16_000),
            sample_rate_out: parsed("SAMPLE_RATE_OUT", 24_000),
            max_audio_seconds: parsed("MAX_AUDIO_SECONDS", 90),
            max_frames: parsed("MAX_FRAMES", 3),
            max_recent_turns: parsed("MAX_RECENT_TURNS", 8),
            context_max_chars: parsed("CONTEXT_MAX_CHARS", 12_000),
            max_tool_rounds: parsed("MAX_TOOL_ROUNDS", 6),
            skills_dir: PathBuf::from(value("SKILLS_DIR", "skills")),
            cli_max_output_bytes: parsed("CLI_MAX_OUTPUT_BYTES", 256 * 1024),
            search_proxy: value("SEARCH_PROXY", ""),
            invite_codes: value("INVITE_CODES", "")
                .split(',')
                .map(str::trim)
                .filter(|code| !code.is_empty())
                .map(str::to_owned)
                .collect(),
            invite_max_uses: parsed("INVITE_MAX_USES", 10),
            invite_ttl_hours: parsed("INVITE_TTL_HOURS", 24 * 7),
            auth_token_ttl_hours: parsed("AUTH_TOKEN_TTL_HOURS", 24 * 30),
            request_timeout: Duration::from_secs(parsed("REQUEST_TIMEOUT_SECONDS", 180)),
            gate_timeout: Duration::from_millis(parsed("GATE_TIMEOUT_MS", 3_000)),
            readiness_timeout: Duration::from_secs(2),
        })
    }
}
