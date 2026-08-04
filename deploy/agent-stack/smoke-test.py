#!/usr/bin/env python3
import argparse
import asyncio
import base64
import json
import os
import sqlite3
import subprocess
import sys
import time
import uuid
from typing import Optional
from pathlib import Path
from urllib.parse import urlencode

REALTIME_PROTOCOL_VERSION = 3


TERMINAL_TYPES = {"response.done", "response.cancelled", "response.failed"}
SMOKE_JPEG_BASE64 = (
    "/9j/4AAQSkZJRgABAQEAYABgAAD/2wBDAP//////////////////////////////////////////////"
    "////////////////////////////////////2wBDAf//////////////////////////////////////////////"
    "////////////////////////////////////wAARCAABAAEDASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAA"
    "X/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/9oADAMBAAIQAxAAAAH/xAAUEAEAAAAAAAAAAAAAAAAAAAAA/9oACAEBAAEF"
    "Aqf/xAAUEQEAAAAAAAAAAAAAAAAAAAAA/9oACAEDAQE/Aaf/xAAUEQEAAAAAAAAAAAAAAAAAAAAA/9oACAECAQE/Aaf/"
    "xAAUEAEAAAAAAAAAAAAAAAAAAAAA/9oACAEBAAY/Ap//xAAUEAEAAAAAAAAAAAAAAAAAAAAA/9oACAEBAAE/IX//2gAM"
    "AwEAAgADAAAAEP/EABQRAQAAAAAAAAAAAAAAAAAAABD/2gAIAQMBAT8QH//EABQRAQAAAAAAAAAAAAAAAAAAABD/2gAI"
    "AQIBAT8QH//EABQQAQAAAAAAAAAAAAAAAAAAABD/2gAIAQEAAT8QH//Z"
)


def smoke_runtime_candidates(
    repository_root: Path, configured_python: str
) -> list[Path]:
    candidates = [Path(configured_python)] if configured_python else []
    candidates.extend(
        [
            Path(sys.executable),
            repository_root / ".venv-qwen3-asr-1.7b/bin/python",
            repository_root / ".venv-qwen3-tts-12hz-1.7b-customvoice/bin/python",
            repository_root / ".venv-qwen3.5-35b-a3b/bin/python",
        ]
    )
    return list(dict.fromkeys(candidates))


def smoke_runtime_has_dependencies(python: Path) -> bool:
    if not python.is_file():
        return False
    result = subprocess.run(
        [str(python), "-c", "import httpx, numpy, websockets"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    return result.returncode == 0


def ensure_smoke_runtime() -> None:
    repository_root = Path(__file__).resolve().parents[2]
    candidates = smoke_runtime_candidates(
        repository_root, os.environ.get("RIPPLE_SMOKE_PYTHON", "").strip()
    )
    if smoke_runtime_has_dependencies(candidates[0]):
        return
    for candidate in candidates[1:]:
        if smoke_runtime_has_dependencies(candidate):
            os.execv(
                str(candidate),
                [str(candidate), str(Path(__file__).resolve()), *sys.argv[1:]],
            )
    raise RuntimeError(
        "smoke test needs httpx, numpy, and websockets; set RIPPLE_SMOKE_PYTHON "
        "to a Python runtime that provides them"
    )


def require_function_call(payload: dict) -> dict:
    calls = [item for item in payload.get("output", []) if item.get("type") == "function_call"]
    if len(calls) != 1:
        raise RuntimeError(f"expected exactly one function_call, got {len(calls)}")
    call = calls[0]
    if not call.get("call_id"):
        raise RuntimeError("function_call did not include call_id")
    arguments = call.get("arguments")
    if not isinstance(arguments, str):
        raise RuntimeError("function_call arguments must be a JSON string")
    json.loads(arguments)
    if "<tool_call>" in arguments:
        raise RuntimeError("function_call arguments still contain tagged tool text")
    return call


def response_output_text(payload: dict) -> str:
    parts = []
    for item in payload.get("output", []):
        if item.get("type") != "message":
            continue
        for content in item.get("content", []):
            if content.get("type") == "output_text" and content.get("text"):
                parts.append(content["text"])
    return "".join(parts).strip()


async def check_responses_tool_loop() -> None:
    import httpx

    agent_url = os.environ.get(
        "RIPPLE_SMOKE_AGENT_URL", "http://127.0.0.1:8712/v1/responses"
    )
    model = os.environ.get("RIPPLE_AGENT_MODEL", "Qwen3.5-35B-A3B")
    tool = {
        "type": "function",
        "name": "calculate",
        "description": "Evaluate arithmetic",
        "parameters": {
            "type": "object",
            "properties": {"expression": {"type": "string"}},
            "required": ["expression"],
            "additionalProperties": False,
        },
    }
    first_request = {
        "model": model,
        "instructions": "You must call calculate exactly once. Do not answer directly.",
        "input": "Calculate 7 * 8 using the calculate function.",
        "tools": [tool],
        "tool_choice": "auto",
        "temperature": 0,
        "max_output_tokens": 128,
    }
    async with httpx.AsyncClient(timeout=60) as client:
        first_response = await client.post(agent_url, json=first_request)
        first_response.raise_for_status()
        first = first_response.json()
        call = require_function_call(first)
        arguments = json.loads(call["arguments"])
        if arguments != {"expression": "7 * 8"}:
            raise RuntimeError(f"unexpected calculate arguments: {arguments}")
        continuation = {
            "model": model,
            "input": first["output"]
            + [
                {
                    "type": "function_call_output",
                    "call_id": call["call_id"],
                    "output": json.dumps({"ok": True, "result": 56}),
                }
            ],
            "temperature": 0,
            "max_output_tokens": 64,
        }
        final_response = await client.post(agent_url, json=continuation)
        final_response.raise_for_status()
        final = final_response.json()
    text = response_output_text(final)
    if not text:
        raise RuntimeError("Responses tool continuation did not return output_text")
    print(f"responses tool loop: ok ({call['call_id']})")
    print(f"responses final text: {text}")


def build_realtime_url(server: str, access_token: str) -> str:
    query = urlencode({"access_token": access_token})
    return f"ws://{server}/v1/agent/realtime?{query}"


def voice_turn_events(turn_id: str, audio: bytes) -> list[dict]:
    return [
        {"type": "input.speech_started", "turn_id": turn_id},
        {
            "type": "input.audio.append",
            "audio": base64.b64encode(audio).decode("ascii"),
            "sample_rate": 16_000,
        },
        {"type": "input.commit", "turn_id": turn_id},
    ]


def requested_frame_events(response_id: str) -> tuple[dict, dict]:
    return (
        {
            "type": "input.video.frame",
            "response_id": response_id,
            "image": SMOKE_JPEG_BASE64,
            "mime_type": "image/jpeg",
            "captured_at": int(time.time() * 1000),
        },
        {"type": "input.video.commit", "response_id": response_id},
    )


def check_terminal_event(
    event: dict,
    response_id: str,
    terminal_response_ids: Optional[set[str]] = None,
) -> Optional[str]:
    event_type = event.get("type")
    if event_type not in TERMINAL_TYPES:
        return None
    event_response_id = event.get("response_id", "")
    if response_id and event_response_id != response_id:
        raise RuntimeError(
            f"terminal event response mismatch: expected {response_id}, got {event_response_id}"
        )
    if terminal_response_ids is not None:
        if event_response_id in terminal_response_ids:
            raise RuntimeError(f"duplicate terminal event for {event_response_id}")
        terminal_response_ids.add(event_response_id)
    if event_type == "response.failed":
        raise RuntimeError(event.get("message") or "response failed")
    return event_type


def check_response_milestones(database_path: str, response_id: str) -> None:
    required = {
        "server.agent.first_delta",
        "server.tts.first_audio",
        "server.output.playback.started",
    }
    with sqlite3.connect(f"file:{database_path}?mode=ro", uri=True) as connection:
        rows = connection.execute(
            "SELECT DISTINCT kind FROM events "
            "WHERE json_extract(payload, '$.response_id') = ?",
            (response_id,),
        ).fetchall()
    found = {row[0] for row in rows}
    missing = required - found
    if missing:
        raise RuntimeError(f"missing milestones: {', '.join(sorted(missing))}")


async def wait_for_gate_decision(database_path: str, session_id: str) -> dict:
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        with sqlite3.connect(f"file:{database_path}?mode=ro", uri=True) as connection:
            row = connection.execute(
                "SELECT payload FROM events WHERE session_id = ? "
                "AND kind = 'server.gate.completed' ORDER BY created_at DESC LIMIT 1",
                (session_id,),
            ).fetchone()
        if row:
            return json.loads(row[0])
        await asyncio.sleep(0.25)
    raise RuntimeError("response Gate did not record a decision")


async def synthesize_probe(text: str) -> bytes:
    import httpx

    tts_url = os.environ.get(
        "RIPPLE_SMOKE_TTS_URL", "http://127.0.0.1:8723/v1/audio/speech"
    )
    payload = {
        "model": os.environ.get(
            "RIPPLE_TTS_MODEL", "Qwen3-TTS-12Hz-1.7B-CustomVoice"
        ),
        "input": text,
        "voice": os.environ.get("RIPPLE_TTS_VOICE", "serena"),
        "language": "Chinese",
        "response_format": "pcm",
        "stream": True,
        "stream_format": "audio",
    }
    async with httpx.AsyncClient(timeout=60) as client:
        response = await client.post(tts_url, json=payload)
        response.raise_for_status()
    if not response.content:
        raise RuntimeError("TTS did not return Gate probe audio")
    return response.content


async def receive_frame_request(socket, terminal_response_ids: set[str]) -> str:
    while True:
        try:
            event = json.loads(await asyncio.wait_for(socket.recv(), timeout=90))
        except asyncio.TimeoutError as error:
            raise RuntimeError("timed out waiting for requested video frame") from error
        if event.get("type") == "error":
            raise RuntimeError(event.get("message") or "gateway returned an error")
        if event.get("type") == "input.frame.requested":
            response_id = event.get("response_id")
            if not response_id:
                raise RuntimeError("requested video frame did not include response_id")
            return response_id
        terminal = check_terminal_event(event, "", terminal_response_ids)
        if terminal:
            raise RuntimeError(f"response ended before requesting video frame: {terminal}")


async def main() -> None:
    import httpx
    import numpy as np
    import websockets

    server = os.environ.get("RIPPLE_SMOKE_SERVER", "127.0.0.1:8700")
    access_token = os.environ.get("RIPPLE_SMOKE_ACCESS_TOKEN", "").strip()
    if not access_token:
        raise RuntimeError("RIPPLE_SMOKE_ACCESS_TOKEN is required")
    endpoints = {
        "gateway liveness": f"http://{server}/health",
        "gateway readiness": f"http://{server}/ready",
    }
    async with httpx.AsyncClient(timeout=10) as client:
        for name, url in endpoints.items():
            response = await client.get(url)
            response.raise_for_status()
            print(f"{name}: healthy")

    url = build_realtime_url(server, access_token)
    received_types: list[str] = []
    audio_parts: list[bytes] = []
    final_text = ""
    asr_transcript = ""
    first_text_at: float | None = None
    first_audio_at: float | None = None
    response_id = ""
    cancelled_response_id = ""
    resumed_response_id = ""
    terminal_response_ids: set[str] = set()
    async with websockets.connect(url, max_size=128 * 1024 * 1024) as socket:
        created = json.loads(await socket.recv())
        assert created["type"] == "session.created"
        session_id = created["session_id"]
        await socket.send(
            json.dumps(
                {
                    "type": "session.start",
                    "mode": "video",
                    "protocol_version": REALTIME_PROTOCOL_VERSION,
                    "client_build": "smoke-test",
                }
            )
        )
        ready = json.loads(await socket.recv())
        assert ready["type"] == "session.ready"
        assert ready["protocol_version"] == REALTIME_PROTOCOL_VERSION
        committed_at = time.perf_counter()
        await socket.send(
            json.dumps(
                {
                    "type": "input.text.commit",
                    "text": "请务必调用 calculate 工具计算 123 乘以 456，然后告诉我结果。",
                },
                ensure_ascii=False,
            )
        )
        while True:
            event = json.loads(await socket.recv())
            received_types.append(event["type"])
            if event["type"] == "response.created":
                response_id = event.get("response_id", "")
            if event["type"].startswith("response.") and event.get("response_id"):
                assert event["response_id"] == response_id
            if event["type"] == "response.text.delta" and first_text_at is None:
                first_text_at = time.perf_counter()
            if event["type"] == "response.audio.delta":
                if first_audio_at is None:
                    first_audio_at = time.perf_counter()
                    await socket.send(
                        json.dumps(
                            {
                                "type": "output.playback.started",
                                "response_id": response_id,
                                "buffered_ms": 450,
                            }
                        )
                    )
                audio_parts.append(base64.b64decode(event["audio"]))
            if event["type"] == "error":
                raise RuntimeError(event["message"])
            terminal = check_terminal_event(event, response_id, terminal_response_ids)
            if terminal == "response.done":
                final_text = event["text"]
                break

        output_samples = np.frombuffer(b"".join(audio_parts), dtype="<f4")
        gate_probe = (
            np.frombuffer(
                await synthesize_probe("今天天气挺好的，我们一会儿去吃饭吧。"),
                dtype="<i2",
            ).astype(np.float32)
            / 32768.0
        )
        target_size = round(gate_probe.size * 16000 / 24000)
        input_samples = np.interp(
            np.linspace(0, gate_probe.size, target_size, endpoint=False),
            np.arange(gate_probe.size),
            gate_probe,
        ).astype("<f4")
        for event in voice_turn_events(str(uuid.uuid4()), input_samples.tobytes()):
            await socket.send(json.dumps(event))
        events_db = os.environ.get(
            "RIPPLE_SMOKE_EVENTS_DB", "runtime-data/agent-gateway/context.sqlite3"
        )
        gate = await wait_for_gate_decision(events_db, session_id)
        if gate.get("gate_decision") != "ignore":
            raise RuntimeError(f"expected unrelated speech to be ignored, got: {gate}")
        asr_transcript = str(gate.get("transcript", "")).strip()

        visual_probe = (
            np.frombuffer(
                await synthesize_probe("请告诉我你看到了什么？"), dtype="<i2"
            ).astype(np.float32)
            / 32768.0
        )
        visual_input_size = round(visual_probe.size * 16000 / 24000)
        visual_input = np.interp(
            np.linspace(0, visual_probe.size, visual_input_size, endpoint=False),
            np.arange(visual_probe.size),
            visual_probe,
        ).astype("<f4")
        for event in voice_turn_events(str(uuid.uuid4()), visual_input.tobytes()):
            await socket.send(json.dumps(event))
        video_response_id = await receive_frame_request(socket, terminal_response_ids)
        for event in requested_frame_events(video_response_id):
            await socket.send(json.dumps(event))
        video_playback_reported = False
        while True:
            event = json.loads(await socket.recv())
            if event.get("type") == "error":
                raise RuntimeError(event.get("message") or "gateway returned an error")
            if event.get("response_id") and event["response_id"] != video_response_id:
                raise RuntimeError(
                    "video response mismatch: "
                    f"expected {video_response_id}, got {event['response_id']}"
                )
            if event.get("type") == "response.audio.delta" and not video_playback_reported:
                video_playback_reported = True
                await socket.send(
                    json.dumps(
                        {
                            "type": "output.playback.started",
                            "response_id": video_response_id,
                            "buffered_ms": 450,
                        }
                    )
                )
            if (
                check_terminal_event(
                    event, video_response_id, terminal_response_ids
                )
                == "response.done"
            ):
                break
        if not video_playback_reported:
            raise RuntimeError("video response did not produce audio")

        await socket.send(
            json.dumps(
                {
                    "type": "input.text.commit",
                    "text": "请详细介绍实时语音 Agent 的组成、工作流程和常见优化方法。",
                },
                ensure_ascii=False,
            )
        )
        while True:
            event = json.loads(await socket.recv())
            if event["type"] == "response.created":
                cancelled_response_id = event["response_id"]
            if event["type"] == "response.audio.delta":
                await socket.send(json.dumps({"type": "response.cancel"}))
            if event["type"] == "error":
                raise RuntimeError(event["message"])
            if (
                check_terminal_event(event, cancelled_response_id, terminal_response_ids)
                == "response.cancelled"
            ):
                assert event["response_id"] == cancelled_response_id
                break

        await socket.send(
            json.dumps(
                {"type": "input.text.commit", "text": "只回答：打断后已恢复。"},
                ensure_ascii=False,
            )
        )
        while True:
            event = json.loads(await socket.recv())
            if event["type"] == "response.created":
                resumed_response_id = event["response_id"]
            if event["type"] == "error":
                raise RuntimeError(event["message"])
            if (
                check_terminal_event(event, resumed_response_id, terminal_response_ids)
                == "response.done"
            ):
                assert event["response_id"] == resumed_response_id
                break

    assert "response.tool.completed" in received_types
    assert response_id
    assert first_text_at is not None
    assert first_audio_at is not None
    assert cancelled_response_id
    assert resumed_response_id
    assert resumed_response_id != cancelled_response_id
    assert len(terminal_response_ids) == 4
    assert output_samples.size > 0
    assert asr_transcript
    assert "<asr_text>" not in asr_transcript
    assert not asr_transcript.lower().startswith("language ")
    check_response_milestones(
        os.environ.get(
            "RIPPLE_SMOKE_EVENTS_DB", "runtime-data/agent-gateway/context.sqlite3"
        ),
        response_id,
    )
    print("protocol 3: ok")
    print("model Gate ignored unrelated speech: ok")
    print("on-demand JPEG video frame: ok")
    print("first-result milestones: ok")
    print("tool loop: ok")
    print("barge-in and response isolation: ok")
    print(f"first text: {first_text_at - committed_at:.3f}s")
    print(f"first audio: {first_audio_at - committed_at:.3f}s")
    print(f"audio chunks: {len(audio_parts)}")
    print(f"audio transport: {output_samples.nbytes} bytes")
    print(f"ASR loopback: {asr_transcript}")
    print(f"answer: {final_text}")


if __name__ == "__main__":
    ensure_smoke_runtime()
    parser = argparse.ArgumentParser()
    parser.add_argument("--responses-only", action="store_true")
    arguments = parser.parse_args()
    asyncio.run(check_responses_tool_loop() if arguments.responses_only else main())
