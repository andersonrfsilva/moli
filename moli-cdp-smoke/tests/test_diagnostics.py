from __future__ import annotations

import asyncio
import json
import tempfile
import unittest
from pathlib import Path
from typing import Any
from types import SimpleNamespace
from unittest.mock import AsyncMock, patch

from moli_cdp_smoke.assertions import wait_until
from moli_cdp_smoke.diagnostics import (
    Diagnostics, ObservedCdpSession, current_diagnostics, diagnostics_scope,
    record_phase, record_wire, write_artifact,
)
from moli_cdp_smoke.raw_cdp import RawCdpClient


class _Socket:
    def __init__(self) -> None:
        self.sent: list[dict[str, Any]] = []
        self.messages: asyncio.Queue[str] = asyncio.Queue()

    async def send(self, message: str) -> None:
        self.sent.append(json.loads(message))

    async def recv(self) -> str:
        return await self.messages.get()


class DiagnosticsTests(unittest.IsolatedAsyncioTestCase):
    async def test_raw_messages_keep_wire_order_and_exact_pending_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            result = Path(directory) / "raw.json"
            with diagnostics_scope(result) as diagnostics:
                socket = _Socket()
                client = RawCdpClient(socket)  # type: ignore[arg-type]
                request = await client.send("Page.navigate", {"url": "http://fixture.test/"},
                                            session_id="page")
                checkpoint = diagnostics.directory / "checkpoint.json"
                await wait_until(lambda: checkpoint.exists()
                                 and json.loads(checkpoint.read_text())["messageCount"] >= 1,
                                 "diagnostic checkpoint", timeout_ms=2_000)
                saved = json.loads(checkpoint.read_text())
                self.assertEqual(saved["pendingCommands"][0]["message"]["id"], request)
                self.assertEqual(saved["pendingCommands"][0]["message"]["sessionId"], "page")
                for message in [{"method": "Page.frameNavigated", "sessionId": "page"},
                                {"id": request, "sessionId": "page", "result": {}}]:
                    socket.messages.put_nowait(json.dumps(message))
                await client.recv_until_id(request)
                client.mark_no_followup_boundary()
                diagnostics.finish()
                saved = json.loads(checkpoint.read_text())
                self.assertEqual(len(socket.sent), 1, "diagnostics sent browser commands")
                self.assertEqual([m["direction"] for m in saved["lastMessages"]],
                                 ["send", "receive", "receive"])
                self.assertEqual(saved["pendingCommands"], [])

    async def test_failure_phase_and_bounded_tail_survive_without_worker_completion(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with diagnostics_scope(Path(directory) / "worker.json") as diagnostics:
                for index in range(300):
                    record_wire(self, "receive", {"method": "event", "params": {
                        "index": index, "payload": "x" * 100_000,
                    }})
                record_phase("navigation/main-frame", "failed", RuntimeError("target crashed"))
                checkpoint = diagnostics.directory / "checkpoint.json"
                await wait_until(lambda: checkpoint.exists()
                                 and json.loads(checkpoint.read_text())["messageCount"] >= 300,
                                 "worker checkpoint", timeout_ms=2_000)
                saved = json.loads(checkpoint.read_text())
                self.assertEqual(saved["messageCount"], 300)
                self.assertLessEqual(len(saved["lastMessages"]), 128)
                self.assertEqual(saved["lastMessages"][-1]["message"]["params"]["index"], 299)
                self.assertLess(checkpoint.stat().st_size, 500_000)
                self.assertTrue(any(phase.get("error") == "RuntimeError: target crashed"
                                    for phase in saved["phases"]))
                self.assertIn("cpuCount", saved["initialResources"])

    async def test_raw_trace_artifacts_are_complete_and_outlive_scope(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            trace = {"traceEvents": [{"name": f"event-{index}"} for index in range(200)]}
            with diagnostics_scope(Path(directory) / "tracing.json"):
                path = write_artifact("cpu-trace.json", trace)
            self.assertEqual(json.loads(Path(path).read_text()), trace)

    async def test_capture_errors_are_retained_as_failures(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            with diagnostics_scope(Path(directory) / "worker.json") as diagnostics:
                with patch.object(Path, "write_text", side_effect=PermissionError("read-only")):
                    self.assertIsNone(write_artifact("cpu-trace.json", {}))
                result = diagnostics.finish()
                self.assertTrue(result["errors"])
                self.assertIn("PermissionError", result["errors"][0])
                self.assertIn("read-only", result["errors"][0])

    async def test_failing_finalization_does_not_leak_the_diagnostic_scope(self) -> None:
        with patch.object(Diagnostics, "finish", side_effect=RuntimeError("serialization failed")):
            with self.assertRaisesRegex(RuntimeError, "serialization failed"):
                with diagnostics_scope(None):
                    self.assertIsNotNone(current_diagnostics())
        self.assertIsNone(current_diagnostics())

    async def test_capture_failure_makes_an_otherwise_successful_worker_fail(self) -> None:
        from moli_cdp_smoke import runner

        fixture = SimpleNamespace(url="http://fixture.test", start=lambda: None, stop=lambda: None)
        diagnostics = SimpleNamespace(finish=lambda: {"errors": ["PermissionError: read-only"]})

        async def smoke(endpoint: str, server: Any, selection: Any, results: list[Any]) -> None:
            results.append({"name": "protocol assertion", "ok": True})

        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "worker.json"
            args = runner.parse_args(["--group", "tracing", "--browser", "chromium",
                                      "--endpoint", "http://fixture.test", "--result", str(path)])
            with (patch.object(runner, "FixtureServer", return_value=fixture),
                  patch.object(runner, "wait_for_cdp_server", new_callable=AsyncMock),
                  patch.object(runner, "browser_identity", new_callable=AsyncMock,
                               return_value={"engine": "chromium"}),
                  patch.object(runner, "stop_browser", new_callable=AsyncMock),
                  patch.object(runner, "run_smoke", side_effect=smoke),
                  patch.object(runner, "current_diagnostics", return_value=diagnostics)):
                code = await runner._run_worker(args)
            saved = json.loads(path.read_text())
            self.assertEqual(code, 1)
            self.assertFalse(saved["ok"])
            self.assertTrue(saved["results"][0]["ok"])
            self.assertIn("diagnostic capture failed", saved["error"])
            self.assertIn("PermissionError", saved["error"])


class _Session:
    def __init__(self) -> None:
        self.calls: list[tuple[str, Any]] = []
        self.result = {"result": {"value": 42}}
        self.error: BaseException | None = None

    async def send(self, method: str, params: Any = None) -> Any:
        self.calls.append((method, params))
        if self.error:
            raise self.error
        return self.result

    async def detach(self) -> None:
        self.calls.append(("detach", None))


class SessionObservationTests(unittest.IsolatedAsyncioTestCase):
    async def test_sdk_calls_keep_their_original_result_and_exception(self) -> None:
        original = _Session()
        session = ObservedCdpSession(original)
        with diagnostics_scope(None) as diagnostics:
            self.assertIs(await session.send("Runtime.evaluate", {"expression": "21 * 2"}),
                          original.result)
            error = ValueError("protocol error")
            original.error = error
            with self.assertRaises(ValueError) as caught:
                await session.send("Page.navigate", {"url": "http://fixture.test/"})
            self.assertIs(caught.exception, error)
            await session.detach()
            self.assertEqual(len(original.calls), 3)
            self.assertEqual(diagnostics.pending, {})
            self.assertEqual([row["direction"] for row in diagnostics.messages],
                             ["send", "receive"] * 3)
            self.assertTrue(all(row["transport"] == "playwright" for row in diagnostics.messages))


if __name__ == "__main__":
    unittest.main()
