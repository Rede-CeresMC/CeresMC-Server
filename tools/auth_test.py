"""With xbox-auth=on, a login without a valid Mojang token must be refused."""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client

c = Client("BotFake")
try:
    c.join(timeout=8)
    spawned = True
except RuntimeError:
    spawned = False
kicked = any(p == 0x05 for p, _ in c.received)
reason = [d for p, d in c.received if p == 0x05]
ok = not spawned and kicked
print(("  ok   " if ok else "  FAIL ") + "a fake Xbox login is refused" + (f" ({reason[-1][3:80]!r})" if reason else ""))
c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
