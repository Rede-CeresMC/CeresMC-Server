"""Smoke tests against a running server (python tools/smoke_client.py).

1. one bot logs in and spawns
2. a second bot joins: both must see each other, movement and chat
"""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

a = Client("BotA")
print("ping:", a.ping())
a.join()
check(a.spawned, f"BotA spawned after {a.chunks} chunks")
check(0xbb in [p for p, _ in a.received], "BotA got UpdateAbilities")

b = Client("BotB")
b.join()
check(b.spawned, f"BotB spawned after {b.chunks} chunks")
a.pump(1.0)
b.pump(1.0, lambda: any(p == 0x0c for p, _ in b.received))
check(any(p == 0x3f for p, _ in b.received), "BotB got the player list")
check(any(p == 0x0c and b"BotA" in d for p, d in b.received), "BotB sees BotA (AddPlayer)")
check(any(p == 0x0c and b"BotB" in d for p, d in a.received), "BotA sees BotB (AddPlayer)")

a.move(260.5, 72.0, 260.5, yaw=90.0)
b.pump(1.0, lambda: any(p == 0x13 for p, _ in b.received))
check(any(p == 0x13 for p, _ in b.received), "BotB sees BotA move")

a.send_text("ola mundo")
b.pump(1.0, lambda: any(p == 0x09 and b"ola mundo" in d for p, d in b.received))
check(any(p == 0x09 and b"ola mundo" in d for p, d in b.received), "BotB gets BotA's chat")

a.close()
b.pump(2.0, lambda: any(p == 0x0e for p, _ in b.received))
check(any(p == 0x0e for p, _ in b.received), "BotB sees BotA leave (RemoveActor)")
b.close()

print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
