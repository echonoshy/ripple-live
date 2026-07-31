import json
import platform
import socket
import sys


def main() -> None:
    payload = json.load(sys.stdin)
    if payload:
        raise ValueError("system_info does not accept arguments")
    print(
        json.dumps(
            {
                "ok": True,
                "data": {
                    "hostname": socket.gethostname(),
                    "os": platform.system(),
                    "kernel": platform.release(),
                    "architecture": platform.machine(),
                },
            },
            ensure_ascii=False,
        )
    )


if __name__ == "__main__":
    main()
