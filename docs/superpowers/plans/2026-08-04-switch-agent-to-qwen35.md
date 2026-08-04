# Switch Agent to Qwen3.5-35B-A3B Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Serve `Qwen3.5-35B-A3B` as the gateway's main Agent on the existing port 8712.

**Architecture:** Keep the public gateway at port 8700 and its internal Agent URL at `http://127.0.0.1:8712/v1/responses`. Replace the vLLM process behind 8712 and set the gateway request model to the served 35B model name.

**Tech Stack:** Bash deployment scripts, user systemd transient units, vLLM, Rust gateway.

## Global Constraints

- The Agent endpoint remains on `127.0.0.1:8712`.
- The Gateway remains on `0.0.0.0:8700`.
- Use the existing 35B runtime `.venv-qwen3.5-35b-a3b` and GPUs `4,5`.

---

### Task 1: Switch the Agent deployment configuration

**Files:**

- Modify: `deploy/agent-stack/start.sh`
- Modify: `deploy/agent-stack/.env`
- Modify: `deploy/agent-stack/.env.example`

**Interfaces:**

- Consumes: `AGENT_MODEL`, `AGENT_RUNTIME`, `AGENT_TOOL_CALL_PARSER`, `RIPPLE_AGENT_URL`, and `RIPPLE_AGENT_MODEL` environment variables.
- Produces: a vLLM Agent service on port 8712 serving `Qwen3.5-35B-A3B` and a Gateway configured to call that same model name.

- [x] **Step 1: Set the durable 35B defaults**

Set `AGENT_MODEL=Qwen3.5-35B-A3B`, `AGENT_RUNTIME=.venv-qwen3.5-35b-a3b`, and `AGENT_TOOL_CALL_PARSER=qwen3_coder`; retain port 8712.

- [ ] **Step 2: Restart the two dependent units**

Run `systemctl --user stop ripple-agent-gateway.service ripple-agent-agent.service`, then run `deploy/agent-stack/start.sh` to recreate the Agent and Gateway units.

- [ ] **Step 3: Verify the endpoint contract**

Run `curl -fsS http://127.0.0.1:8712/v1/models` and confirm its sole model id is `Qwen3.5-35B-A3B`; run `curl -fsS http://127.0.0.1:8700/health` to confirm the gateway process returned.
