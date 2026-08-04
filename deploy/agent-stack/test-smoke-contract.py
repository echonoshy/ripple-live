import importlib.util
import base64
import json
import os
import stat
import sqlite3
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("ripple_smoke", ROOT / "smoke-test.py")
assert SPEC and SPEC.loader
SMOKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SMOKE)


class SmokeContractTests(unittest.TestCase):
    def test_start_uses_the_runtime_that_contains_qwen_asr_serve(self) -> None:
        start_script = (ROOT / "start.sh").read_text(encoding="utf-8")
        self.assertIn(
            'ASR_RUNTIME="${ASR_RUNTIME:-$REPO_ROOT/.venv-qwen3-asr-1.7b}"',
            start_script,
        )

    def test_deployment_defaults_do_not_restore_retired_model_weights(self) -> None:
        download_script = (ROOT / "download-models.sh").read_text(encoding="utf-8")
        example_environment = (ROOT / ".env.example").read_text(encoding="utf-8")
        smoke_script = (ROOT / "smoke-test.py").read_text(encoding="utf-8")

        self.assertNotIn("Qwen3-ASR-0.6B", download_script)
        self.assertNotIn("Qwen3-TTS-12Hz-0.6B-CustomVoice", download_script)
        self.assertNotIn("Qwen3-VL-8B-Instruct", download_script)
        self.assertIn("RIPPLE_ASR_MODEL=Qwen3-ASR-1.7B", example_environment)
        self.assertIn('"Qwen3.5-35B-A3B"', smoke_script)

    def test_gateway_state_defaults_to_runtime_data_away_from_model_cache(self) -> None:
        start_script = (ROOT / "start.sh").read_text(encoding="utf-8")
        example_environment = (ROOT / ".env.example").read_text(encoding="utf-8")
        smoke_script = (ROOT / "smoke-test.py").read_text(encoding="utf-8")
        gateway_config = (
            ROOT.parent.parent / "services" / "agent-gateway" / "src" / "config.rs"
        ).read_text(encoding="utf-8")

        self.assertIn("RIPPLE_DATA_DIR=runtime-data/agent-gateway", example_environment)
        self.assertIn(
            "RIPPLE_TOOL_CACHE_DB=runtime-data/agent-gateway/tool-cache.sqlite3",
            example_environment,
        )
        self.assertIn(
            'GATEWAY_DATA_DIR="${RIPPLE_DATA_DIR:-$REPO_ROOT/runtime-data/agent-gateway}"',
            start_script,
        )
        self.assertIn('"RIPPLE_SMOKE_EVENTS_DB", "runtime-data/agent-gateway/context.sqlite3"', smoke_script)
        self.assertIn('value("DATA_DIR", "runtime-data/agent-gateway")', gateway_config)

    def test_realtime_smoke_uses_protocol_three(self) -> None:
        self.assertEqual(SMOKE.REALTIME_PROTOCOL_VERSION, 3)
        smoke_source = (ROOT / "smoke-test.py").read_text(encoding="utf-8")
        self.assertNotIn("protocol 2", smoke_source)

    def test_responses_tool_call_rejects_tagged_arguments(self) -> None:
        item = SMOKE.require_function_call(
            {
                "output": [
                    {
                        "type": "function_call",
                        "name": "calculate",
                        "call_id": "call_1",
                        "arguments": '{"expression":"7 * 8"}',
                    }
                ]
            }
        )
        self.assertEqual(item["name"], "calculate")
        self.assertEqual(json.loads(item["arguments"]), {"expression": "7 * 8"})
        self.assertNotIn("<tool_call>", item["arguments"])

    def test_realtime_url_encodes_access_token(self) -> None:
        self.assertEqual(
            SMOKE.build_realtime_url("127.0.0.1:8700", "a+b/c="),
            "ws://127.0.0.1:8700/v1/agent/realtime?access_token=a%2Bb%2Fc%3D",
        )

    def test_smoke_runtime_prefers_an_explicit_python(self) -> None:
        candidates = SMOKE.smoke_runtime_candidates(
            Path("/repo"), "/custom/smoke-python"
        )

        self.assertEqual(candidates[0], Path("/custom/smoke-python"))
        self.assertIn(Path("/repo/.venv-qwen3-asr-1.7b/bin/python"), candidates)

    def test_voice_turn_events_reuse_one_turn_id(self) -> None:
        events = SMOKE.voice_turn_events("turn-7", b"\0\0\0\0")

        self.assertEqual(
            events,
            [
                {"type": "input.speech_started", "turn_id": "turn-7"},
                {
                    "type": "input.audio.append",
                    "audio": "AAAAAA==",
                    "sample_rate": 16_000,
                },
                {"type": "input.commit", "turn_id": "turn-7"},
            ],
        )

    def test_requested_frame_events_are_valid_jpeg_and_correlated(self) -> None:
        frame, commit = SMOKE.requested_frame_events("response-9")

        self.assertEqual(frame["response_id"], "response-9")
        self.assertEqual(frame["mime_type"], "image/jpeg")
        self.assertTrue(base64.b64decode(frame["image"]).startswith(b"\xff\xd8"))
        self.assertEqual(
            commit,
            {"type": "input.video.commit", "response_id": "response-9"},
        )

    def test_failed_response_is_a_terminal_error(self) -> None:
        with self.assertRaisesRegex(RuntimeError, "Agent 服务暂时不可用"):
            SMOKE.check_terminal_event(
                {
                    "type": "response.failed",
                    "response_id": "response-1",
                    "message": "Agent 服务暂时不可用",
                },
                "response-1",
            )
        self.assertEqual(
            SMOKE.check_terminal_event(
                {"type": "response.done", "response_id": "response-1"},
                "response-1",
            ),
            "response.done",
        )

    def test_status_reports_liveness_and_readiness_separately(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            bin_dir = Path(directory)
            systemctl = bin_dir / "systemctl"
            curl = bin_dir / "curl"
            systemctl.write_text("#!/bin/sh\nexit 1\n", encoding="utf-8")
            curl.write_text(
                "#!/bin/sh\ncase \"$*\" in *'/ready'*) exit 22;; *) exit 0;; esac\n",
                encoding="utf-8",
            )
            systemctl.chmod(systemctl.stat().st_mode | stat.S_IXUSR)
            curl.chmod(curl.stat().st_mode | stat.S_IXUSR)
            environment = os.environ.copy()
            environment["PATH"] = f"{bin_dir}:/usr/bin:/bin"

            result = subprocess.run(
                ["bash", str(ROOT / "status.sh")],
                check=True,
                capture_output=True,
                text=True,
                env=environment,
            )

        self.assertIn("gateway liveness: ok", result.stdout)
        self.assertIn("gateway readiness: unavailable", result.stdout)

    def test_milestones_require_agent_audio_and_playback_events(self) -> None:
        with tempfile.NamedTemporaryFile() as database:
            with sqlite3.connect(database.name) as connection:
                connection.execute(
                    "CREATE TABLE events(session_id TEXT, kind TEXT, payload TEXT, created_at REAL)"
                )
                for kind in (
                    "server.agent.first_delta",
                    "server.tts.first_audio",
                    "server.output.playback.started",
                ):
                    connection.execute(
                        "INSERT INTO events VALUES(?, ?, ?, 0)",
                        ("session-1", kind, '{"response_id":"response-1"}'),
                    )

            SMOKE.check_response_milestones(database.name, "response-1")
            with self.assertRaisesRegex(RuntimeError, "missing milestones"):
                SMOKE.check_response_milestones(database.name, "response-2")


if __name__ == "__main__":
    unittest.main()
