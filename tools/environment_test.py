"""Environmental damage (python tools/environment_test.py): standing in lava hurts and sets the
player on fire."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, block_id, uvarint, string, read_uvarint

STONE = block_id("minecraft:stone")
X, Y, Z = 304, 70, 266

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + bytes(1) + string(""))
    c.pump(0.6)

def health(c, since):
    val = None
    for pid, d in c.received[since:]:
        if pid != 0x1d:
            continue
        _, p = read_uvarint(d, 0)
        n, p = read_uvarint(d, p)
        for _ in range(n):
            vals = struct.unpack("<6f", d[p:p + 24]); p += 24
            ln, p = read_uvarint(d, p)
            nm = d[p:p + ln].decode(); p += ln
            _, p = read_uvarint(d, p)
            if nm == "minecraft:health":
                val = vals[2]
    return val

def on_fire_seen(c, since):
    """True if any SetActorData carried the ONFIRE flag."""
    for pid, d in c.received[since:]:
        if pid != 0x27:
            continue
        _, p = read_uvarint(d, 0)
        n, p = read_uvarint(d, p)
        key, p = read_uvarint(d, p)
        _, p = read_uvarint(d, p); p += 1
        flags, p = read_uvarint(d, p) if False else (None, p)
        # FLAGS is a varlong: decode it
        v, shift = 0, 0
        while True:
            b = d[p]; p += 1
            v |= (b & 0x7f) << shift; shift += 7
            if not b & 0x80:
                break
        v = (v >> 1) ^ -(v & 1)
        if key == 0 and v & 1:
            return True
    return False

c = Client("BotLava")
c.join()
c.move(X + 0.5, Y + 1.62, Z + 0.5)
c.pump(0.5)
command(c, "/gamemode creative")
for dy in (0, 1):
    c.break_block((X, Y + dy, Z))
c.place_block((X, Y - 1, Z), 1, STONE)
c.pump(0.6)
command(c, "/gamemode survival")
command(c, "/clear")
n0 = len(c.received)
command(c, "/give @s lava_bucket 1")
c.pump(2.0, lambda: any(p == 0x32 for p, _ in c.received[n0:]))
c.move(X + 0.5, Y + 1.62, Z + 0.5)
n = len(c.received)
c.place_block((X, Y - 1, Z), 1, 0)  # lava at the feet
c.pump(2.0)
hp = health(c, n)
check(hp is not None and hp < 20, f"standing in lava hurts ({hp})")
check(on_fire_seen(c, n), "the player is set on fire (ONFIRE flag)")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
