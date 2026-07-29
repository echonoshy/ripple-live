# MiniCPM-o 4.5 Realtime Service

This deployment runs the official PyTorch realtime service on physical GPU 1
and exposes its plain WebSocket gateway on host port 8600.

## Architecture

- Public gateway: `http://0.0.0.0:8600`
- Audio: `ws://140.143.229.103:8600/v1/realtime?mode=audio`
- Video: `ws://140.143.229.103:8600/v1/realtime?mode=video`
- Worker and backend ports remain inside the Compose network.
- Model weights are mounted read-only from `.cache/models/MiniCPM-o-4_5`.

## Prepare

```bash
./deploy/realtime-o45/setup.sh
```

The helper downloads a pinned copy of the official realtime service into the
ignored `.cache/services` directory.

## Start with Docker

The current user must be able to access `/var/run/docker.sock`.

```bash
./deploy/realtime-o45/start.sh
```

## Status and logs

```bash
./deploy/realtime-o45/status.sh
./deploy/realtime-o45/compose.sh -f deploy/realtime-o45/docker-compose.yml logs -f
```

## Stop

```bash
./deploy/realtime-o45/stop.sh
```

## Start without Docker

Use the bare-metal launcher when the current account cannot access Docker:

```bash
./deploy/realtime-o45/setup-baremetal.sh
./deploy/realtime-o45/start-baremetal.sh
./deploy/realtime-o45/status-baremetal.sh
```

It binds only the public gateway to `0.0.0.0:8600`. The worker and model
backend bind to loopback. On this server the launcher uses transient user
systemd services so the processes remain alive after the shell exits. Stop it
with:

```bash
./deploy/realtime-o45/stop-baremetal.sh
```

This service intentionally uses unencrypted `ws://` for the requested initial
deployment. Audio, images, prompts, and responses are visible in transit.
