use std::{collections::BTreeMap, time::Instant};

use serde::Serialize;

use crate::{config::Settings, context::ContextStore};

#[derive(Clone, Debug, Serialize)]
pub struct DependencyReadiness {
    pub ok: bool,
    pub elapsed_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReadinessReport {
    pub ok: bool,
    pub dependencies: BTreeMap<String, DependencyReadiness>,
}

pub async fn check(settings: &Settings, context: &ContextStore) -> ReadinessReport {
    let client = reqwest::Client::builder()
        .timeout(settings.readiness_timeout)
        .build()
        .expect("readiness HTTP client configuration is valid");
    let (asr, agent, tts, database) = tokio::join!(
        probe_http(
            &client,
            &settings.asr_backend,
            &settings.asr_readiness_url,
            None
        ),
        probe_http(
            &client,
            &settings.agent_backend,
            &settings.agent_readiness_url,
            Some(&settings.agent_api_key),
        ),
        probe_http(
            &client,
            &settings.tts_backend,
            &settings.tts_readiness_url,
            None
        ),
        probe_database(context),
    );
    let dependencies = BTreeMap::from([
        ("agent".to_owned(), agent),
        ("asr".to_owned(), asr),
        ("database".to_owned(), database),
        ("tts".to_owned(), tts),
    ]);
    ReadinessReport {
        ok: dependencies.values().all(|dependency| dependency.ok),
        dependencies,
    }
}

async fn probe_http(
    client: &reqwest::Client,
    backend: &str,
    url: &str,
    bearer_token: Option<&str>,
) -> DependencyReadiness {
    let started = Instant::now();
    if backend == "mock" {
        return ready(started);
    }
    let mut request = client.get(url);
    if let Some(token) = bearer_token {
        request = request.bearer_auth(token);
    }
    match request.send().await {
        Ok(response) if response.status().is_success() => ready(started),
        Ok(_) => failed(started, "http_status"),
        Err(error) if error.is_timeout() => failed(started, "timeout"),
        Err(_) => failed(started, "unreachable"),
    }
}

async fn probe_database(context: &ContextStore) -> DependencyReadiness {
    let started = Instant::now();
    match context.readiness().await {
        Ok(()) => ready(started),
        Err(_) => failed(started, "database"),
    }
}

fn ready(started: Instant) -> DependencyReadiness {
    DependencyReadiness {
        ok: true,
        elapsed_ms: started.elapsed().as_millis(),
        error: None,
    }
}

fn failed(started: Instant, error: &'static str) -> DependencyReadiness {
    DependencyReadiness {
        ok: false,
        elapsed_ms: started.elapsed().as_millis(),
        error: Some(error),
    }
}
