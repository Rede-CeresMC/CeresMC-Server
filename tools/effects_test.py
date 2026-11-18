"""Status effects (python tools/effects_test.py): /effect adds and removes effects, Speed changes
the movement attribute, Poison hurts over time in survival but never kills."""
import sys, os, struct
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from bedrock_client import Client, uvarint, string, read_uvarint

ok = True
def check(cond, what):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what)
    ok &= bool(cond)

def command(c, line):
    c.send(uvarint(0x4d) + string(line) + string("player") + bytes(16) + string("") + struct.pack("<q", 0) + bytes(1) + string(""))
    c.pump(0.6)

def zz(v):
    return (v >> 1) ^ -(v & 1)

def effects(c, since):
    """[(event, effect, amplifier)] of the MobEffect packets received."""
    out = []
    for pid, d in c.received[since:]:
        if pid != 0x1c:
            continue
        _, p = read_uvarint(d, 0)
        event = d[p]; p += 1
        eff, p = read_uvarint(d, p)
        amp, p = read_uvarint(d, p)
        out.append((event, zz(eff), zz(amp)))
    return out

def attribute(c, name, since=0):
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
            if nm == name:
                val = vals[2]
    return val

c = Client("BotEffect")
c.join()
c.pump(0.5)
command(c, "/gamemode survival")

n = len(c.received)
command(c, "/effect @s speed 10 1")
check((1, 1, 1) in effects(c, n), f"/effect speed sends MobEffect add ({effects(c, n)})")
mv = attribute(c, "minecraft:movement", n)
check(mv is not None and abs(mv - 0.14) < 0.001, f"Speed II raises the movement attribute ({mv})")

n = len(c.received)
command(c, "/effect @s poison 30 3")
c.pump(4.0)
hp = attribute(c, "minecraft:health", n)
check(hp is not None and 1.0 <= hp < 20.0, f"Poison hurts over time ({hp})")
c.pump(3.0)
hp = attribute(c, "minecraft:health", n)
check(hp is not None and hp >= 1.0, f"Poison never kills ({hp})")

n = len(c.received)
command(c, "/effect @s clear")
ev = effects(c, n)
check(any(e[0] == 3 and e[1] == 19 for e in ev) and any(e[0] == 3 and e[1] == 1 for e in ev), f"/effect clear removes them ({ev})")

n = len(c.received)
command(c, "/effect @s instant_health 1 1")
hp2 = attribute(c, "minecraft:health", n)
check(hp2 is not None and hp2 >= min(20.0, (hp or 1) + 8), f"Instant Health II heals 8 ({hp} -> {hp2})")

c.close()
print("PASS" if ok else "FAILED")
sys.exit(0 if ok else 1)
