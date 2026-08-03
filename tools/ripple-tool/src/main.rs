mod cache;
mod contract;
mod http;
mod weather;
mod web;

use std::io::{Read, Write};

use clap::{Parser, Subcommand};
use contract::{ToolError, failure, success};

#[derive(Parser)]
#[command(
    name = "ripple-tool",
    version,
    about = "Ripple Live read-only tool CLI"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, Subcommand)]
enum Command {
    WebSearch,
    WebFetch,
    WeatherLookup,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let output = match read_input() {
        Ok(input) => match cli.command {
            Command::WebSearch => match serde_json::from_value::<web::SearchInput>(input) {
                Ok(input) => match web::WebClient::from_env() {
                    Ok(client) => client
                        .search(input)
                        .await
                        .map(|output| success(output.data, output.meta)),
                    Err(error) => Err(error),
                },
                Err(error) => Err(ToolError::invalid(format!("参数格式错误: {error}"))),
            },
            Command::WebFetch => match serde_json::from_value::<web::FetchInput>(input) {
                Ok(input) => match web::WebClient::from_env() {
                    Ok(client) => client
                        .fetch(input)
                        .await
                        .map(|output| success(output.data, output.meta)),
                    Err(error) => Err(error),
                },
                Err(error) => Err(ToolError::invalid(format!("参数格式错误: {error}"))),
            },
            Command::WeatherLookup => {
                match serde_json::from_value::<weather::WeatherInput>(input) {
                    Ok(input) => match weather::WeatherClient::from_env() {
                        Ok(client) => client
                            .lookup(input)
                            .await
                            .map(|output| success(output.data, output.meta)),
                        Err(error) => Err(error),
                    },
                    Err(error) => Err(ToolError::invalid(format!("参数格式错误: {error}"))),
                }
            }
        },
        Err(error) => Err(error),
    };
    let value = output.unwrap_or_else(failure);
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, &value).expect("serializing a JSON value cannot fail");
    stdout.write_all(b"\n").expect("stdout write failed");
}

fn read_input() -> Result<serde_json::Value, ToolError> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(64 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|_| ToolError::invalid("无法读取 stdin"))?;
    if bytes.is_empty() {
        return Err(ToolError::invalid("stdin 必须包含 JSON object"));
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| ToolError::invalid(format!("stdin JSON 无效: {error}")))?;
    if !value.is_object() {
        return Err(ToolError::invalid("stdin 必须是 JSON object"));
    }
    Ok(value)
}
