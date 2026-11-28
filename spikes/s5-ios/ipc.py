"""Spike S5: one IPC call to an agent's Unix socket. Usage: ipc.py <socket> <call> [json]"""
import json, socket, sys
s = socket.socket(socket.AF_UNIX); s.connect(sys.argv[1])
s.sendall((json.dumps({"id": 1, "call": sys.argv[2],
                       "params": json.loads(sys.argv[3]) if len(sys.argv) > 3 else {}}) + "\n").encode())
f = s.makefile()
for line in f:
    v = json.loads(line)
    if v.get("id") == 1:
        print(json.dumps(v.get("result") if v.get("ok") else {"error": v.get("error")}))
        break
