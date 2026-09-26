#!/usr/bin/env python3
"""NX04 offline fixture: separate native producer/consumer, no model.

Use NX01's strict JSON decoder and status vocabulary; its fixed three-arm
Quartz answer oracle does not cover these checkpoint and revision scenarios.
"""

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import sys
import tempfile

from native_eval_scoring import unique_object


TASK = "Quartz-J9"
CASES = ("failed_test", "revised_fact", "dirty_pass", "isolated_project")
DIRECTIONS = ("cli_to_mcp", "stop_to_cli")
MAX_OUTPUT = 65536
TIMEOUT = 20
WORKER_TIMEOUT = 60
WORKER_PROCESS = False


def invoke(command, cwd, env, payload="", timeout=TIMEOUT):
    """Fixed argv only; bounded file output avoids unbounded pipe capture."""
    # Nested native calls stay in the worker group owned by the controller.
    own_group = not WORKER_PROCESS
    with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
        child = subprocess.Popen(command, cwd=cwd, env=env, stdin=subprocess.PIPE,
                                 stdout=out, stderr=err, start_new_session=own_group)
        try:
            child.communicate(payload.encode(), timeout=timeout)
        except subprocess.TimeoutExpired as exc:
            if own_group:
                os.killpg(child.pid, signal.SIGKILL)
            else:
                child.kill()
            child.communicate()
            raise RuntimeError("timeout") from exc
        finally:
            if own_group:
                # Also reap descendants after a worker returns an error early.
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        out.seek(0)
        err.seek(0)
        stdout, stderr = out.read(MAX_OUTPUT + 1), err.read(MAX_OUTPUT + 1)
    if len(stdout) > MAX_OUTPUT or len(stderr) > MAX_OUTPUT:
        raise RuntimeError("output_limit")
    if child.returncode:
        raise RuntimeError(f"native_exit_{child.returncode}")
    return stdout.decode("utf-8")


def cli(binary, project, env, args, value=None):
    return invoke([str(binary), *args], project, env,
                  "" if value is None else json.dumps(value) + "\n")


def rpc(binary, project, env, name, arguments):
    request = {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
               "params": {"name": name, "arguments": arguments}}
    raw = cli(binary, project, env, ["mcp", "serve"], request)
    reply = json.loads(raw, object_pairs_hook=unique_object)
    if reply.get("id") != 1 or reply.get("error") or reply["result"].get("isError"):
        raise RuntimeError("mcp_error")
    return reply["result"]["structuredContent"]


def native_json(binary, project, env, args, value=None):
    return json.loads(cli(binary, project, env, args, value), object_pairs_hook=unique_object)


def project_init(binary, project, env):
    (project / ".git").mkdir(parents=True)
    (project / "service.txt").write_text("fixture state\n")
    cli(binary, project, env, ["init", "--no-agent-files"])
    cli(binary, project, env, ["store-upgrade", "--commit"])


def git_commit(project, env):
    git = "/usr/bin/git"
    for args in (["init", "-q"], ["add", "service.txt"],
                 ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.test",
                  "commit", "-qm", "initial fixture"]):
        invoke([git, *args], project, env)


def producer(binary, project, env, direction, case):
    project_init(binary, project, env)
    if case == "dirty_pass":
        git_commit(project, env)
    source = "native-cli-fixture" if direction == "cli_to_mcp" else "native-stop-fixture"
    other = project.parent / "other-project"
    if case == "isolated_project":
        project_init(binary, other, env)
    write_project = other if case == "isolated_project" else project
    old = "Quartz-J9 uses SQLite on port 7111." if case == "revised_fact" else \
          "Quartz-J9 uses SQLite on port 7429."
    if direction == "stop_to_cli":
        # The native Stop/host distiller persists the actual service fact.
        config = write_project / ".mnemosyne" / "config.toml"
        config.write_text("[distill]\nenabled=true\nengine='host'\nsession_summary=false\n")
        transcript = write_project / "session.jsonl"
        block = ("**Findings:**\n- type: codebase\n- importance: 70\n"
                 "- title: Quartz-J9 service\n- content: |\n    " + old + "\n")
        transcript.write_text(json.dumps({"type": "assistant", "message": {
            "role": "assistant", "content": [{"type": "text", "text": block}]}}) + "\n")
        stop_payload = {"transcript_path": str(transcript), "session_id": "nx04-stop"}
        cli(binary, write_project, env, ["hook", "Stop"], stop_payload)
        stop_hits = native_json(binary, write_project, env,
                                ["search", "Quartz-J9", "--scope", "project", "--format", "json"])
        if len(stop_hits) != 1 or old not in stop_hits[0]["summary"]:
            raise RuntimeError("stop_saved_nothing")
        first_stop = native_json(binary, write_project, env,
                                 ["show-v2", stop_hits[0]["id"]])
        replay = cli(binary, write_project, env, ["hook", "Stop"], stop_payload)
        second_stop = native_json(binary, write_project, env,
                                  ["show-v2", stop_hits[0]["id"]])
        if replay.strip() or first_stop["provenance"]["source_events"] != \
                second_stop["provenance"]["source_events"]:
            raise RuntimeError("stop_replay_wrote_source")
        transcript.unlink()
        reference = first_stop["memory_ref"]
        fact_key = first_stop["memory"]["extra"]["fact_key"]
    request = {"type": "codebase", "title": "Quartz-J9 service", "content": old,
               "importance": 70, "origin": source, "source_session_id": "nx04-producer",
               "source_event_id": f"{case}-first", "finding_key": "service",
               "fact_key": f"nx04-{case}-service" if direction == "cli_to_mcp" else fact_key,
               "source_kind": "code_observation", "verification_state": "unverified"}
    if direction == "cli_to_mcp":
        memory = native_json(binary, write_project, env, ["write-v2"], request)
        reference = memory["memory_ref"]
    revision = 1
    if case == "revised_fact":
        first = native_json(binary, project, env, ["show-v2", reference["memory_id"]])
        changed = native_json(binary, project, env, ["revise-v2"], {
            "memory_ref": reference, "expected_rev": 1,
            "expected_hash": first["revision"]["semantic_hash"],
            "changes": {"body": "## Quartz-J9 service\n\nQuartz-J9 uses SQLite on port 7429."}})
        revision = changed["revision"]["semantic_rev"]
        request.update(content="Quartz-J9 uses SQLite on port 7429.",
                       source_event_id=f"{case}-confirmed")
        corroborated = native_json(binary, project, env, ["write-v2"], request)
        if corroborated["memory_ref"] != reference:
            raise RuntimeError("revision_source_unbound")
        revision = native_json(binary, project, env,
                               ["show-v2", reference["memory_id"]])["revision"]["semantic_rev"]
    failure = None
    if case == "failed_test":
        failure = subprocess.run([sys.executable, "-c", "assert 1 == 2"],
                                 cwd=project, env=env, timeout=TIMEOUT, check=False).returncode
        if failure != 1:
            raise RuntimeError("fixture_failure_not_observed")
    if case == "dirty_pass":
        success = subprocess.run([sys.executable, "-c", "assert 1 == 1"],
                                 cwd=project, env=env, timeout=TIMEOUT, check=False).returncode
        if success:
            raise RuntimeError("fixture_pass_not_observed")
    checkpoint_id = None
    if case != "isolated_project":
        fingerprint = native_json(binary, project, env,
                                  ["checkpoint", "observe", "--paths", "service.txt"])["fingerprint"]
        tests = []
        if case in ("failed_test", "dirty_pass"):
            tests.append({"command": "python -c 'assert 1 == 2'" if case == "failed_test" else
                                     "python -c 'assert 1 == 1'", "cwd": ".",
                          "worktree_fingerprint": fingerprint,
                          "exit_code": failure if case == "failed_test" else 0,
                          "executed_at": datetime.now(timezone.utc).isoformat()})
        checkpoint = native_json(binary, project, env, ["checkpoint", "new"], {
            "task_id": TASK, "goal": "Continue Quartz-J9 verification",
            "source_agent": source, "source_session": "nx04-producer",
            "scoped_paths": ["service.txt"], "tests": tests,
            "unresolved": ["synthetic test failed"] if case == "failed_test" else [],
            "next_action": "Fix the failing synthetic test" if case == "failed_test" else
                           "Revalidate after source change" if case == "dirty_pass" else
                           "Check the revised source", "expires": "2099-01-01T00:00:00Z"})
        checkpoint_id = checkpoint["id"]
        if case == "dirty_pass":
            (project / "service.txt").write_text("fixture state changed after test\n")
    return {"memory_ref": reference, "revision": revision,
            "checkpoint_id": checkpoint_id, "observed_failure": failure,
            "stop_saved": direction == "stop_to_cli"}


def validate_receipt(case, receipt):
    if not isinstance(receipt, dict) or not receipt.get("memory_ref") or \
            (case != "isolated_project" and not receipt.get("checkpoint_id")):
        raise RuntimeError("producer_saved_nothing")


def verify_persistence(binary, project, env, case, receipt):
    """Exit 0 and plausible JSON are insufficient without native durable reads."""
    validate_receipt(case, receipt)
    source_project = project.parent / "other-project" if case == "isolated_project" else project
    memory_id = receipt["memory_ref"]["memory_id"]
    shown = native_json(binary, source_project, env, ["show-v2", memory_id])
    if shown.get("memory_ref") != receipt["memory_ref"]:
        raise RuntimeError("producer_saved_nothing")
    if case != "isolated_project":
        loaded = native_json(binary, project, env,
                             ["checkpoint", "load", receipt["checkpoint_id"]])
        if loaded.get("checkpoint", {}).get("id") != receipt["checkpoint_id"]:
            raise RuntimeError("producer_saved_nothing")


def summarize(rows):
    complete = (len(rows) == len(CASES) * len(DIRECTIONS) and
                {(r.get("direction"), r.get("case")) for r in rows} ==
                {(d, c) for d in DIRECTIONS for c in CASES} and
                all(r.get("invocation_status") == "completed" for r in rows))
    return {"execution_complete": complete,
            "acceptance_passed": complete and all(
                r.get("protocol_status") == "valid" and r.get("quality_status") == "passed"
                for r in rows)}


def consumer(binary, project, env, direction):
    """Only task name and own project path cross the process boundary."""
    if direction == "cli_to_mcp":
        bundle = rpc(binary, project, env, "mnemosyne_prep_context",
                     {"version": 2, "project_path": str(project), "task": TASK,
                      "task_id": TASK, "budget": 1200})
    else:
        bundle = native_json(binary, project, env,
                             ["prep", TASK, "--task-id", TASK,
                              "--budget", "1200", "--format", "json"])
    memories, checkpoints = [], []
    for item in bundle["items"]:
        kind, item_id = item.get("kind"), item["id"]
        if kind == "checkpoint":
            view = (rpc(binary, project, env, "mnemosyne_show",
                        {"version": 2, "kind": "checkpoint", "id": item_id,
                         "project_path": str(project)}) if direction == "cli_to_mcp" else
                    native_json(binary, project, env, ["checkpoint", "load", item_id]))
            checkpoints.append(view)
        elif kind == "memory" or item.get("scope") == "project":
            view = (rpc(binary, project, env, "mnemosyne_show",
                        {"version": 2, "kind": "memory", "id": item_id,
                         "project_path": str(project)}) if direction == "cli_to_mcp" else
                    native_json(binary, project, env, ["show-v2", item_id]))
            memories.append(view)
    answer = {"next_action": checkpoints[0]["checkpoint"]["data"]["next_action"]
              if checkpoints else "unknown"}
    return {"memories": memories, "checkpoints": checkpoints, "answer": answer,
            "context_bytes": len(bundle["context"].encode())}


def score(case, receipt, observed, canary, invocation_status="completed"):
    row = {"case": case, "invocation_status": invocation_status,
           "protocol_status": "not_checked", "quality_status": "not_scored",
           "memory_ref": receipt.get("memory_ref"), "revision": receipt.get("revision"),
           "checkpoint_id": receipt.get("checkpoint_id")}
    if invocation_status != "completed":
        return row
    if (not isinstance(observed, dict) or
            set(observed) != {"memories", "checkpoints", "answer", "context_bytes"} or
            not isinstance(observed["memories"], list) or
            not isinstance(observed["checkpoints"], list) or
            not isinstance(observed["answer"], dict) or
            not isinstance(observed["answer"].get("next_action"), str) or
            not isinstance(observed["context_bytes"], int)):
        row["protocol_status"] = "bad_schema"
        return row
    if any(not isinstance(m, dict) or not isinstance(m.get("revision"), dict) or
           not isinstance(m.get("memory"), dict) or
           not isinstance(m["memory"].get("body"), str) or
           not isinstance(m.get("provenance"), dict)
           for m in observed["memories"]) or any(
               not isinstance(c, dict) or not isinstance(c.get("checkpoint"), dict) or
               not isinstance(c.get("test_results"), list)
               for c in observed["checkpoints"]):
        row["protocol_status"] = "bad_schema"
        return row
    if observed["context_bytes"] > 4800:
        row["protocol_status"] = "budget_exceeded"
        return row
    row["protocol_status"] = "valid"
    memories, checkpoints = observed["memories"], observed["checkpoints"]
    if canary in json.dumps(observed):
        row["protocol_status"] = "canary_leak"
        return row
    ref = receipt["memory_ref"]
    matching = [m for m in memories if isinstance(m, dict) and m.get("memory_ref") == ref]
    cp = [c for c in checkpoints if isinstance(c, dict) and
          isinstance(c.get("checkpoint"), dict) and
          c["checkpoint"].get("id") == receipt.get("checkpoint_id")]
    if case == "isolated_project":
        passed = not memories and not checkpoints and observed["answer"]["next_action"] == "unknown"
    else:
        expected_rev = 3 if case == "revised_fact" else 1
        passed = (len(matching) == len(cp) == 1 and
                  matching[0].get("revision", {}).get("semantic_rev") == expected_rev and
                  receipt.get("revision") == expected_rev and
                  "SQLite on port 7429" in matching[0].get("memory", {}).get("body", "") and
                  "port 7111" not in matching[0].get("memory", {}).get("body", ""))
        if passed:
            events = matching[0]["provenance"].get("source_events", [])
            passed = bool(events) and events[0].get("origin") == (
                "claude-code" if receipt.get("stop_saved") else "native-cli-fixture")
        if passed:
            view = cp[0]
            data = view["checkpoint"]["data"]
            if case == "failed_test":
                passed = (receipt["observed_failure"] == 1 and
                          data["unresolved"] == ["synthetic test failed"] and
                          data["next_action"] == "Fix the failing synthetic test" and
                          observed["answer"]["next_action"] == "Fix the failing synthetic test" and
                          view["test_results"][0]["status"] == "reported_fail")
            elif case == "dirty_pass":
                passed = (view["worktree_matches"] is False and
                          view["test_results"][0]["status"] == "reported_pass" and
                          view["test_results"][0]["needs_revalidation"] is True and
                          view["checkpoint"]["worktree"]["observed_commit"] is not None and
                          observed["answer"]["next_action"] == "Revalidate after source change" and
                          data["next_action"] == "Revalidate after source change")
            else:
                passed = (observed["answer"]["next_action"] == "Check the revised source" and
                          data["next_action"] == "Check the revised source" and
                          matching[0]["provenance"]["source_revisions"] == [1, 2])
    row["quality_status"] = "passed" if passed else "failed"
    row["consumer"] = observed
    return row


def worker(args):
    global WORKER_PROCESS
    WORKER_PROCESS = True
    binary, project = Path(args.binary).resolve(), Path(args.project).resolve()
    env = {"HOME": str(project.parent / "home"),
           "MNEMOSYNE_HOME": str(project.parent / "global"),
           "PATH": str(project.parent / "git-bin"),
           "TMPDIR": str(project.parent / "tmp")}
    if args.worker == "produce":
        return producer(binary, project, env, args.direction, args.case)
    return consumer(binary, project, env, args.direction)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--worker", choices=("produce", "consume"))
    parser.add_argument("--project", type=Path)
    parser.add_argument("--direction", choices=DIRECTIONS)
    parser.add_argument("--case", choices=CASES)
    args = parser.parse_args()
    if args.worker:
        print(json.dumps(worker(args)))
        return 0
    binary = args.binary.resolve()
    rows = []
    canary = secrets.token_hex(24)
    with tempfile.TemporaryDirectory(prefix="mnemosyne-nx04-") as tmp:
        root = Path(tmp)
        for direction in DIRECTIONS:
            for case in CASES:
                base = root / direction / case
                project = base / "project"
                project.mkdir(parents=True)
                (base / "tmp").mkdir()
                (base / "git-bin").mkdir()
                (base / "git-bin" / "git").symlink_to("/usr/bin/git")
                env = {"PATH": str(base / "git-bin"), "HOME": str(base / "worker-home"),
                       "MNEMOSYNE_HOME": str(base / "worker-global"),
                       "TMPDIR": str(base / "tmp")}
                def run_worker(mode):
                    argv = [sys.executable, str(Path(__file__).resolve()), str(binary),
                            "--worker", mode, "--project", str(project),
                            "--direction", direction]
                    if mode == "produce":
                        argv += ["--case", case]
                    return json.loads(invoke(argv, base, env, timeout=WORKER_TIMEOUT),
                                      object_pairs_hook=unique_object)
                try:
                    receipt = run_worker("produce")
                    # A successful process with no persisted record cannot count as execution.
                    verify_persistence(binary, project, env, case, receipt)
                    observation = run_worker("consume")
                    row = score(case, receipt, observation, canary)
                except (RuntimeError, ValueError, KeyError, IndexError, TypeError,
                        UnicodeError, OSError) as exc:
                    row = {"case": case, "invocation_status": "failed",
                           "protocol_status": "not_checked", "quality_status": "not_scored",
                           "reason": str(exc)}
                row["direction"] = direction
                rows.append(row)
    report = {"schema_version": 1, "lane": "NX04_OFFLINE_NATIVE_CONTRACT",
              "scope": "simulated fixtures; native CLI/MCP; no model or live host claim",
              "evaluation_mode": "offline_native_contract",
              "producer_kind": "deterministic_process",
              "consumer_kind": "deterministic_process",
              "model_evaluation": "NOT_RUN", "live_host_evaluation": "NOT_RUN",
              "limitations": ["temporary HOME/store isolation is not an OS sandbox",
                              "no real model reasoning or authenticated host session"],
              **summarize(rows), "runs": rows}
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k != "runs"}))
    return 0 if report["acceptance_passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
