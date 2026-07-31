#!/usr/bin/env python3
import argparse
import asyncio
import io
import statistics
import time
import wave

import httpx

TEXTS = [
    "第一路并发测试，现在开始生成语音。",
    "第二路并发测试，验证动态批处理能力。",
    "第三路并发测试，观察服务延迟是否稳定。",
    "第四路并发测试，所有请求应该同时完成。",
    "第五路并发测试，检查排队和批次切换。",
    "第六路并发测试，持续生成清晰的中文语音。",
    "第七路并发测试，记录总体吞吐和实时率。",
    "第八路并发测试，完成本轮服务压力检查。",
]


async def synthesize(
    client: httpx.AsyncClient,
    url: str,
    model: str,
    voice: str,
    instructions: str,
    stream: bool,
    index: int,
    text: str,
) -> tuple[float, float, float | None]:
    started = time.perf_counter()
    payload = {
        "model": model,
        "input": text,
        "voice": voice,
        "language": "Chinese",
        "instructions": instructions,
        "response_format": "pcm" if stream else "wav",
        "stream": stream,
    }
    if stream:
        payload["stream_format"] = "audio"
    first_audio: float | None = None
    if stream:
        total_bytes = 0
        async with client.stream("POST", url, json=payload) as response:
            response.raise_for_status()
            async for chunk in response.aiter_bytes():
                if chunk and first_audio is None:
                    first_audio = time.perf_counter() - started
                total_bytes += len(chunk)
        latency = time.perf_counter() - started
        duration = total_bytes / 2 / 24_000
    else:
        response = await client.post(url, json=payload)
        response.raise_for_status()
        latency = time.perf_counter() - started
        with wave.open(io.BytesIO(response.content), "rb") as audio:
            assert audio.getframerate() == 24_000
            assert audio.getnchannels() == 1
            duration = audio.getnframes() / audio.getframerate()
    ttfa = f" ttfa={first_audio:.3f}s" if first_audio is not None else ""
    print(
        f"request {index}: latency={latency:.3f}s "
        f"audio={duration:.3f}s rtf={latency / duration:.3f}{ttfa}"
    )
    return latency, duration, first_audio


async def run(args: argparse.Namespace) -> None:
    texts = [TEXTS[index % len(TEXTS)] for index in range(args.concurrency)]
    async with httpx.AsyncClient(timeout=args.timeout) as client:
        started = time.perf_counter()
        results = await asyncio.gather(
            *(
                synthesize(
                    client,
                    args.url,
                    args.model,
                    args.voice,
                    args.instructions,
                    args.stream,
                    index + 1,
                    text,
                )
                for index, text in enumerate(texts)
            )
        )
        wall_time = time.perf_counter() - started

    latencies = [result[0] for result in results]
    audio_seconds = sum(result[1] for result in results)
    first_audio_times = [result[2] for result in results if result[2] is not None]
    sorted_latencies = sorted(latencies)
    p95_index = min(len(sorted_latencies) - 1, round(len(sorted_latencies) * 0.95))
    print(
        f"wall={wall_time:.3f}s "
        f"p50={statistics.median(latencies):.3f}s "
        f"p95={sorted_latencies[p95_index]:.3f}s "
        f"throughput={audio_seconds / wall_time:.2f} audio-sec/sec"
    )
    if first_audio_times:
        print(f"ttfa median={statistics.median(first_audio_times):.3f}s")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--url",
        default="http://127.0.0.1:8723/v1/audio/speech",
    )
    parser.add_argument(
        "--model",
        default="Qwen3-TTS-12Hz-1.7B-CustomVoice",
    )
    parser.add_argument("--voice", default="serena")
    parser.add_argument(
        "--instructions",
        default="自然、温暖、亲切的中文女声。语速适中，语气平静，停顿自然，像真人助手交流，避免播音腔和夸张情绪。",
    )
    parser.add_argument("--stream", action="store_true")
    parser.add_argument("--concurrency", type=int, default=4)
    parser.add_argument("--timeout", type=float, default=180)
    args = parser.parse_args()
    if args.concurrency < 1:
        parser.error("--concurrency must be positive")
    asyncio.run(run(args))


if __name__ == "__main__":
    main()
