#!/usr/bin/env python3
import asyncio
import base64
import json
import os
import time
import uuid
from pathlib import Path

import httpx
import numpy as np
import websockets


async def main() -> None:
    endpoints = {
        "asr": "http://127.0.0.1:8711/health",
        "agent": "http://127.0.0.1:8712/health",
        "tts": os.environ.get("RIPPLE_TTS_HEALTH_URL", "http://127.0.0.1:8723/health"),
        "gateway": "http://127.0.0.1:8700/health",
    }
    async with httpx.AsyncClient(timeout=10) as client:
        for name, url in endpoints.items():
            response = await client.get(url)
            response.raise_for_status()
            print(f"{name}: healthy")

    session_id = f"smoke-{uuid.uuid4()}"
    url = f"ws://127.0.0.1:8700/v1/agent/realtime?session_id={session_id}"
    received_types: list[str] = []
    audio_parts: list[bytes] = []
    final_text = ""
    asr_transcript = ""
    first_text_at: float | None = None
    first_audio_at: float | None = None
    response_id = ""
    cancelled_response_id = ""
    resumed_response_id = ""
    async with websockets.connect(url, max_size=128 * 1024 * 1024) as socket:
        created = json.loads(await socket.recv())
        assert created["type"] == "session.created"
        await socket.send(json.dumps({"type": "session.start", "mode": "video"}))
        ready = json.loads(await socket.recv())
        assert ready["type"] == "session.ready"
        test_image = Path("apps/android/src-tauri/icons/icon.png").read_bytes()
        committed_at = time.perf_counter()
        await socket.send(
            json.dumps(
                {
                    "type": "input.video.frame",
                    "image": base64.b64encode(test_image).decode("ascii"),
                    "mime_type": "image/png",
                }
            )
        )
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
                audio_parts.append(base64.b64decode(event["audio"]))
            if event["type"] == "error":
                raise RuntimeError(event["message"])
            if event["type"] == "response.done":
                final_text = event["text"]
                break

        output_samples = np.frombuffer(b"".join(audio_parts), dtype="<f4")
        target_size = round(output_samples.size * 16000 / 24000)
        input_samples = np.interp(
            np.linspace(0, output_samples.size, target_size, endpoint=False),
            np.arange(output_samples.size),
            output_samples,
        ).astype("<f4")
        await socket.send(json.dumps({"type": "input.speech_started"}))
        await socket.send(
            json.dumps(
                {
                    "type": "input.audio.append",
                    "audio": base64.b64encode(input_samples.tobytes()).decode("ascii"),
                    "sample_rate": 16000,
                }
            )
        )
        await socket.send(json.dumps({"type": "input.commit"}))
        while True:
            event = json.loads(await socket.recv())
            if event["type"] == "input.transcript.final":
                asr_transcript = event["text"]
            if event["type"] == "error":
                raise RuntimeError(event["message"])
            if event["type"] == "response.done":
                break

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
            if event["type"] == "response.cancelled":
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
            if event["type"] == "response.done":
                assert event["response_id"] == resumed_response_id
                break

    assert "response.tool.completed" in received_types
    assert response_id
    assert first_text_at is not None
    assert first_audio_at is not None
    assert cancelled_response_id
    assert resumed_response_id
    assert resumed_response_id != cancelled_response_id
    assert output_samples.size > 0
    assert asr_transcript
    assert "<asr_text>" not in asr_transcript
    assert not asr_transcript.lower().startswith("language ")
    print("tool loop: ok")
    print("barge-in and response isolation: ok")
    print(f"first text: {first_text_at - committed_at:.3f}s")
    print(f"first audio: {first_audio_at - committed_at:.3f}s")
    print(f"audio chunks: {len(audio_parts)}")
    print(f"audio transport: {output_samples.nbytes} bytes")
    print(f"ASR loopback: {asr_transcript}")
    print(f"answer: {final_text}")


if __name__ == "__main__":
    asyncio.run(main())
